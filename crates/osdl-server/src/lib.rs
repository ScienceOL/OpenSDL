//! gRPC server library for OpenSDL.
//!
//! `serve()` is the entrypoint: it owns the lifecycle of the gRPC service,
//! the per-instance lockfile, and either/both UDS and TCP listeners. It
//! does **not** own the engine — the caller constructs an `OsdlEngine`
//! (and runs its loop), passes the resulting `EngineHandle` here, and
//! shuts the engine down when `serve()` returns.
//!
//! Why split it that way: in the desktop-bundled mode the engine + agent
//! live in the same process, and the agent wants direct `EngineHandle`
//! access without going through gRPC. Decoupling `run-the-engine` from
//! `expose-it-via-gRPC` keeps that mode simple.

pub mod account;
mod account_endpoint;
pub mod auth;
pub mod background;
pub mod cluster;
pub mod convert;
mod lab_profile;
pub mod lockfile;
pub mod managed;
mod media_publish;
pub mod paths;
pub mod service;
pub mod simulation_asset;
mod snapshot;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use osdl_core::EngineHandle;
mod listeners;
pub mod local_access;
use tokio::{
    sync::{watch, Notify},
    task::JoinSet,
};

use cluster::WorkerListener;

pub use account_endpoint::{serve_account_endpoint, MediaRelay};
pub use lockfile::{InstanceRecord, LockfileGuard};
pub use paths::Paths;
pub use service::{OsdlService, ServerIdentity};

/// Crate version baked into the build — surfaced via `Status` and the
/// instance lockfile so clients can warn on version skew.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where to listen.
///
/// `socket_path` is a Unix-only Unix domain socket. On Windows it is
/// silently ignored (the field exists so the struct shape is identical
/// across platforms — Windows builds always use the TCP listener).
#[derive(Debug, Clone, Default)]
pub struct ListenConfig {
    /// Path to a Unix domain socket. If `None`, no UDS listener is started.
    /// On Windows this field is ignored.
    pub socket_path: Option<PathBuf>,
    /// TCP socket address. If `None`, no TCP listener is started.
    pub tcp_addr: Option<SocketAddr>,
}

impl ListenConfig {
    pub fn is_empty(&self) -> bool {
        // On Windows `socket_path` is always ignored, so an "empty" listen
        // config is one with no TCP address regardless of whether the
        // caller bothered to None out socket_path.
        #[cfg(unix)]
        {
            self.socket_path.is_none() && self.tcp_addr.is_none()
        }
        #[cfg(not(unix))]
        {
            self.tcp_addr.is_none()
        }
    }
}

/// Configuration for `serve()`.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Logical name for this server instance. One lockfile per instance.
    pub instance: String,
    /// Where to listen. Both UDS and TCP can be set; either alone is fine.
    pub listen: ListenConfig,
    /// Where to write the instance lockfile (typically `paths.lock_dir()`).
    pub lock_dir: PathBuf,
    pub auth: auth::ListenerAuth,
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("listener config empty: must set at least one of socket_path or tcp_addr")]
    NoListener,
    #[error("authentication configuration: {0}")]
    Auth(String),
    #[error("laboratory profile: {0:#}")]
    LabProfile(#[source] anyhow::Error),
    #[error("lockfile error: {0}")]
    Lock(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
    #[error("gRPC listener task: {0}")]
    ListenerTask(#[from] tokio::task::JoinError),
    #[error("gRPC listener stopped before shutdown")]
    ListenerExited,
}

/// Run the gRPC server until the engine is asked to stop, the process
/// receives a Ctrl-C, or the listener fails. Cleans up the lockfile and
/// any UDS file on the way out.
pub async fn serve(engine: EngineHandle, cfg: ServeConfig) -> Result<(), ServeError> {
    serve_with_worker(engine, cfg, None, None).await
}

/// Attach the cluster ingress to the same engine and publisher registry.
pub async fn serve_with_worker(
    engine: EngineHandle,
    cfg: ServeConfig,
    worker: Option<WorkerListener>,
    initial_lab_name: Option<&str>,
) -> Result<(), ServeError> {
    if cfg.listen.is_empty() {
        return Err(ServeError::NoListener);
    }

    cfg.auth
        .validate_listener(&cfg.instance, cfg.listen.tcp_addr)
        .map_err(ServeError::Auth)?;

    #[cfg(unix)]
    if matches!(
        cfg.auth,
        auth::ListenerAuth::Account(_) | auth::ListenerAuth::PublicKey(_)
    ) && cfg.listen.socket_path.is_none()
    {
        return Err(ServeError::Auth(
            "remote authentication requires a protected local administrator socket".into(),
        ));
    }
    let pid = std::process::id();
    let started_at = convert::now_ts();

    // Bind TCP up-front — non-destructive, and we need the
    // kernel-assigned port (when `--listen 127.0.0.1:0`) to record the
    // real address in the lockfile. If TCP binding fails we haven't
    // touched any shared state yet.
    let tcp_listener = if let Some(addr) = cfg.listen.tcp_addr {
        Some(
            tokio::net::TcpListener::bind(addr)
                .await
                .map_err(ServeError::Io)?,
        )
    } else {
        None
    };
    let bound_tcp_addr = tcp_listener.as_ref().and_then(|l| l.local_addr().ok());

    // UDS is Unix-only. On Windows the field is ignored — even if a
    // caller hands us a `socket_path` we drop it from the lockfile so
    // discovery doesn't advertise an unreachable endpoint.
    #[cfg(unix)]
    let effective_socket_path = cfg.listen.socket_path.clone();
    #[cfg(not(unix))]
    let effective_socket_path: Option<PathBuf> = None;

    let socket_path_str = effective_socket_path
        .as_ref()
        .map(|p| p.display().to_string());
    let listen_addr_str = bound_tcp_addr.as_ref().map(|a| a.to_string());

    // Reserve the lockfile *before* we touch the UDS path. Otherwise a
    // second `lab serve --instance default` could `remove_file` the
    // live server's socket as part of "stale-socket cleanup" before its
    // own lockfile-write fails — leaving the live server with a missing
    // socket. Reserving first means the live PID check rejects the
    // second invocation before it can do any damage.
    let record = InstanceRecord::new(
        cfg.instance.clone(),
        pid,
        SERVER_VERSION,
        effective_socket_path.clone(),
        listen_addr_str.clone(),
    );
    let _lock = lockfile::write(&cfg.lock_dir, &record).map_err(ServeError::Lock)?;
    let lab_profile = engine
        .data_dir()
        .map(|dir| lab_profile::Store::open(dir, &cfg.instance, initial_lab_name))
        .transpose()
        .map_err(ServeError::LabProfile)?;

    // Now safe to unlink the UDS path (it's stale by definition — no
    // live server has the lock), then bind. Failures here propagate
    // naturally; the lockfile guard will clean up on drop.
    #[cfg(unix)]
    let unix_listener = if let Some(ref socket_path) = effective_socket_path {
        let parent = socket_path
            .parent()
            .ok_or_else(|| ServeError::Auth("invalid_socket_path".into()))?;
        osdl_auth::ProfileStore::open(parent.to_path_buf())
            .map_err(|e| ServeError::Auth(format!("Unix socket directory {} must be a real directory owned by the service user with mode 0700; choose a canonical path and create it as that user: {e}", parent.display())))?;
        match std::fs::remove_file(socket_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ServeError::Io(e)),
        }
        let listener = tokio::net::UnixListener::bind(socket_path).map_err(ServeError::Io)?;
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(socket_path, perms).map_err(ServeError::Io)?;
        }
        Some(listener)
    } else {
        None
    };

    let identity = ServerIdentity {
        instance: cfg.instance.clone(),
        version: SERVER_VERSION.to_string(),
        pid,
        started_at,
        socket_path: socket_path_str,
        listen_addr: listen_addr_str,
    };

    let (stop, stopped) = watch::channel(false);
    let shutdown = Arc::new(Notify::new());
    let service = OsdlService::with_shutdown(engine.clone(), identity, shutdown.clone())
        .with_lab_profile(lab_profile);

    let mut joins = JoinSet::new();

    if let Some(listener) = tcp_listener {
        let access = match &cfg.auth {
            auth::ListenerAuth::Local => {
                let access = local_access::LocalAccess::generate(format!(
                    "http://{}",
                    bound_tcp_addr.unwrap()
                ))
                .map_err(|e| ServeError::Auth(e.to_string()))?;
                access
                    .save(&cfg.lock_dir, &cfg.instance)
                    .map_err(|e| ServeError::Auth(e.to_string()))?;
                listeners::TcpAccess::LocalCapability(access.token)
            }
            auth::ListenerAuth::LocalCapability(token) => {
                listeners::TcpAccess::LocalCapability(token.clone())
            }
            _ => listeners::TcpAccess::Remote,
        };
        joins.spawn(listeners::tcp(
            listener,
            service.clone(),
            cfg.auth.clone(),
            cfg.instance.clone(),
            access,
            stopped.clone(),
        ));
    }
    #[cfg(windows)]
    if matches!(
        cfg.auth,
        auth::ListenerAuth::Account(_) | auth::ListenerAuth::PublicKey(_)
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let access =
            local_access::LocalAccess::generate(format!("http://{}", listener.local_addr()?))
                .map_err(|e| ServeError::Auth(e.to_string()))?;
        access
            .save(&cfg.lock_dir, &cfg.instance)
            .map_err(|e| ServeError::Auth(e.to_string()))?;
        joins.spawn(listeners::tcp(
            listener,
            service.clone(),
            cfg.auth.clone(),
            cfg.instance.clone(),
            listeners::TcpAccess::LocalCapability(access.token),
            stopped.clone(),
        ));
    }
    #[cfg(unix)]
    if let Some(listener) = unix_listener {
        joins.spawn(listeners::unix(
            listener,
            service.clone(),
            cfg.auth.clone(),
            stopped.clone(),
        ));
    }
    if let Some(worker) = worker {
        joins.spawn(worker.listen(engine.clone(), service, stopped));
    }
    let stopped = supervise_listeners(engine, shutdown, stop, joins).await;

    // Best-effort cleanup of UDS file (Unix only).
    #[cfg(unix)]
    if let Some(ref p) = effective_socket_path {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("failed to remove UDS file {}: {}", p.display(), e),
        }
    }

    // _lock drops here, removing the lockfile.
    stopped
}

pub(crate) async fn listener_stopped(mut stopped: watch::Receiver<bool>) {
    let _ = stopped.wait_for(|value| *value).await;
}

pub(crate) async fn supervise_listeners(
    engine: EngineHandle,
    shutdown: Arc<Notify>,
    stop: watch::Sender<bool>,
    mut listeners: JoinSet<Result<(), tonic::transport::Error>>,
) -> Result<(), ServeError> {
    let mut result = tokio::select! {
        biased;
        _ = engine.stop_requested() => Ok(()),
        _ = wait_for_console_signal(tokio::signal::ctrl_c()) => Ok(()),
        finished = listeners.join_next() => match finished {
            Some(Ok(Err(error))) => Err(error.into()),
            Some(Err(error)) => Err(error.into()),
            _ => Err(ServeError::ListenerExited),
        },
    };
    // Stop accepting RPCs even if a listener has yet to poll its stop signal.
    stop.send_replace(true);
    engine.request_stop();
    shutdown.notify_waiters();
    shutdown.notify_one();
    while let Some(finished) = listeners.join_next().await {
        let finished = finished
            .map_err(ServeError::from)
            .and_then(|result| result.map_err(Into::into));
        if result.is_ok() {
            result = finished;
        }
    }
    result
}

async fn wait_for_console_signal(signal: impl std::future::Future<Output = std::io::Result<()>>) {
    if let Err(error) = signal.await {
        log::warn!(
            "console signal handler unavailable; waiting for authenticated shutdown: {error}"
        );
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod shutdown_signal_tests {
    #[tokio::test]
    async fn unavailable_console_signals_do_not_shut_down_a_server() {
        let unavailable = async { Err(std::io::Error::other("no attached console")) };
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            super::wait_for_console_signal(unavailable)
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn an_actual_console_signal_completes_shutdown() {
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            super::wait_for_console_signal(async { Ok(()) })
        )
        .await
        .is_ok());
    }
    #[tokio::test]
    async fn a_stop_requested_before_serving_closes_the_service() {
        let temp = tempfile::tempdir().unwrap();
        let engine = osdl_core::OsdlEngine::new(osdl_core::config::OsdlConfig::default(), vec![]);
        let handle = engine.handle();
        // `lab serve` requests a stop when the engine exits during startup,
        // which can happen before the control service subscribes.
        handle.request_stop();
        let config = super::ServeConfig {
            instance: "stopped".into(),
            listen: super::ListenConfig {
                socket_path: None,
                tcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            },
            lock_dir: temp.path().join("locks"),
            auth: super::auth::ListenerAuth::Bearer("test".into()),
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::serve(handle, config),
        )
        .await
        .expect("serve stops for a request made before it started")
        .unwrap();
    }
}
pub mod authorization;
pub mod public_key;
