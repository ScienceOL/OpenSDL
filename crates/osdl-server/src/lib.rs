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
pub mod auth;
pub mod background;
pub mod convert;
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
use tokio::sync::Notify;

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
    #[error("lockfile error: {0}")]
    Lock(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
}

/// Run the gRPC server until the engine is asked to stop, the process
/// receives a Ctrl-C, or the listener fails. Cleans up the lockfile and
/// any UDS file on the way out.
pub async fn serve(engine: EngineHandle, cfg: ServeConfig) -> Result<(), ServeError> {
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

    // One Notify drives all shutdown wake-ups: both the per-listener
    // graceful-shutdown futures *and* every active StreamEvents stream
    // (which would otherwise hold the RPC open until its broadcast
    // sender closes — i.e. forever, since the service still owns it).
    let shutdown = Arc::new(Notify::new());
    let service = OsdlService::with_shutdown(engine.clone(), identity, shutdown.clone());

    let mut joins: Vec<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>> = Vec::new();

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
            _ => listeners::TcpAccess::Remote,
        };
        joins.push(tokio::spawn(listeners::tcp(
            listener,
            service.clone(),
            cfg.auth.clone(),
            cfg.instance.clone(),
            access,
            shutdown.clone(),
        )));
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
        joins.push(tokio::spawn(listeners::tcp(
            listener,
            service.clone(),
            cfg.auth.clone(),
            cfg.instance.clone(),
            listeners::TcpAccess::LocalCapability(access.token),
            shutdown.clone(),
        )));
    }
    #[cfg(unix)]
    if let Some(listener) = unix_listener {
        joins.push(tokio::spawn(listeners::unix(
            listener,
            service.clone(),
            cfg.auth.clone(),
            shutdown.clone(),
        )));
    }
    // Wait for stop signal — engine stop OR Ctrl-C.
    let mut stop = engine.stop_handle().subscribe();
    tokio::select! {
        _ = async {
            // Already requested before we subscribed? Fire immediately.
            if *stop.borrow() {
                return;
            }
            let _ = stop.changed().await;
        } => log::info!("engine stop requested, shutting down listeners"),
        _ = wait_for_console_signal(tokio::signal::ctrl_c()) => log::info!("Ctrl-C received, shutting down"),
    }

    // Notify both listeners. `notify_waiters` would only wake currently-
    // waiting tasks; we use `notify_one` per listener instead so the wake
    // is delivered even if the listener task hasn't reached `notified()`
    // yet (each listener has its own Arc<Notify> clone — wait, they
    // share. So `notify_waiters` works because each listener calls
    // `notified()` immediately after spawn. Use it.).
    shutdown.notify_waiters();
    // Some listeners may not have reached notified() yet; permit storage
    // ensures the next call also wakes.
    shutdown.notify_one();

    // Drain listener tasks. They each return Ok(()) on graceful shutdown.
    for j in joins {
        if let Ok(Err(e)) = j.await {
            log::error!("gRPC listener error during shutdown: {e}");
        }
    }

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
    Ok(())
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
}
pub mod authorization;
pub mod public_key;
