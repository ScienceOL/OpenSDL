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

pub mod auth;
pub mod convert;
pub mod lockfile;
pub mod paths;
pub mod service;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use osdl_core::EngineHandle;
use osdl_proto::v1 as pb;
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
    /// Bearer token required for *TCP* RPCs. UDS clients skip this check
    /// — filesystem perms (mode 0600) are the auth there. `None` means
    /// "no auth", which we only accept when the TCP listener is bound
    /// to a loopback address; binding `0.0.0.0`/non-loopback without a
    /// token fails fast.
    pub auth_token: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("listener config empty: must set at least one of socket_path or tcp_addr")]
    NoListener,
    #[error(
        "TCP listener {addr} is non-loopback but no --auth-token is configured. \
         Set OSDL_AUTH_TOKEN/--auth-token, or bind 127.0.0.1 / ::1 / Unix domain \
         socket only. Hardware control without auth on a routable address is \
         refused."
    )]
    AuthRequired { addr: SocketAddr },
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

    // Hardware control without auth on a routable address is unsafe —
    // anyone on the lab network could drive devices or shut us down.
    // Loopback (127.0.0.0/8 + ::1) is the only address we accept
    // without a token. Refuse the misconfiguration up-front so the
    // lockfile/UDS path never even gets touched.
    if let Some(addr) = cfg.listen.tcp_addr {
        if cfg.auth_token.is_none() && !addr.ip().is_loopback() {
            return Err(ServeError::AuthRequired { addr });
        }
    }

    let pid = std::process::id();
    let started_at = convert::now_ts();

    // Bind TCP up-front — non-destructive, and we need the
    // kernel-assigned port (when `--listen 127.0.0.1:0`) to record the
    // real address in the lockfile. If TCP binding fails we haven't
    // touched any shared state yet.
    let tcp_listener = if let Some(addr) = cfg.listen.tcp_addr {
        Some(tokio::net::TcpListener::bind(addr).await.map_err(ServeError::Io)?)
    } else {
        None
    };
    let bound_tcp_addr = tcp_listener
        .as_ref()
        .and_then(|l| l.local_addr().ok());

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
    // second `osdl serve --instance default` could `remove_file` the
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
        let bound = bound_tcp_addr.expect("listener implies addr");
        let shutdown = shutdown.clone();
        // TCP gets the bearer-token interceptor when a token is set.
        // (The loopback-without-token case is allowed; the unauth check
        // at the top of `serve()` already refuses non-loopback without a
        // token.) UDS skips this — filesystem perms are the auth there.
        let tcp_service = pb::osdl_server::OsdlServer::new(service.clone());
        let token = cfg.auth_token.clone();
        joins.push(tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            let mut builder = tonic::transport::Server::builder();
            match token {
                Some(t) => {
                    let intercepted = tonic::service::interceptor::InterceptedService::new(
                        tcp_service,
                        auth::bearer_interceptor(t),
                    );
                    builder
                        .add_service(intercepted)
                        .serve_with_incoming_shutdown(incoming, async move {
                            shutdown.notified().await;
                        })
                        .await
                }
                None => {
                    builder
                        .add_service(tcp_service)
                        .serve_with_incoming_shutdown(incoming, async move {
                            shutdown.notified().await;
                        })
                        .await
                }
            }
        }));
        log::info!(
            "gRPC TCP listening on {bound}{}",
            if cfg.auth_token.is_some() {
                " (auth: bearer)"
            } else {
                " (no auth — loopback only)"
            }
        );
    }

    #[cfg(unix)]
    if let Some(listener) = unix_listener {
        let socket_path = effective_socket_path
            .clone()
            .expect("listener implies path");
        let shutdown = shutdown.clone();
        let uds_service = pb::osdl_server::OsdlServer::new(service);
        joins.push(tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(uds_service)
                .serve_with_incoming_shutdown(incoming, async move {
                    shutdown.notified().await;
                })
                .await
        }));
        log::info!("gRPC UDS listening at {}", socket_path.display());
    }
    // On non-Unix the original `service` was cloned into the TCP branch
    // and the original is now unused — silence the warning.
    #[cfg(not(unix))]
    let _ = service;

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
        _ = tokio::signal::ctrl_c() => log::info!("Ctrl-C received, shutting down"),
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
