//! `osdl serve` — boot the engine + broker + mDNS, expose gRPC over UDS/TCP.
//!
//! Foreground is the default — that's what process supervisors (launchd,
//! systemd, docker) expect. `--detach` reuses the `daemonize` crate to
//! run as a Unix daemon (double-fork, setsid, stdio redirection) when the
//! user wants the simple "fire and forget" experience without setting up
//! a service file.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use clap::Args;
use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::config::{AdapterConfig, EspNowDongleConfig, MqttConfig, OsdlConfig};
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::{EmbeddedBroker, EventStore, MdnsAdvertiser, OsdlEngine};
use osdl_server::{paths::Paths, ListenConfig, ServeConfig};

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// YAML config file. Falls back to a sensible default config if omitted.
    #[arg(long, env = "OSDL_CONFIG")]
    pub config: Option<PathBuf>,

    /// Logical instance name. Determines the lockfile, default socket path,
    /// and default db filename.
    #[arg(long, env = "OSDL_INSTANCE", default_value = "default")]
    pub instance: String,

    /// Where to bind the Unix domain socket. Default:
    /// `<runtime_dir>/<instance>.sock`. Set `--socket=disabled` to opt
    /// out of UDS entirely (then a TCP listener must be set).
    #[arg(long, env = "OSDL_SOCKET")]
    pub socket: Option<String>,

    /// Bind a TCP gRPC listener. Off by default — UDS only.
    #[arg(long, env = "OSDL_LISTEN")]
    pub listen: Option<SocketAddr>,

    /// Override the state dir (where the SQLite db lives).
    #[arg(long, env = "OSDL_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Override the registry directory for the unilabos adapter. Default
    /// pulls from `OSDL_REGISTRY_PATH` then `registry/unilabos`.
    #[arg(long, env = "OSDL_REGISTRY_PATH")]
    pub registry: Option<PathBuf>,

    /// ESP-NOW dongle port (USB-CDC). Optional.
    #[arg(long, env = "OSDL_DONGLE_PORT")]
    pub dongle_port: Option<String>,

    /// ESP-NOW dongle baud rate (default 115200).
    #[arg(long, env = "OSDL_DONGLE_BAUD", default_value_t = 115200)]
    pub dongle_baud: u32,

    /// Run in the background as a Unix daemon. Stdout/stderr are
    /// redirected to `--log-file`. The current process forks twice,
    /// detaches from the controlling terminal, and exits — leaving the
    /// daemon to run independently of your shell session. Use
    /// `osdl stop --instance NAME` to terminate it.
    ///
    /// Foreground (the default) is preferred under launchd, systemd,
    /// docker, or any other supervisor.
    #[cfg(unix)]
    #[arg(long)]
    pub detach: bool,

    /// Where to redirect stdout/stderr when running with `--detach`.
    /// Default: `<state_dir>/<instance>.log`.
    #[arg(long, env = "OSDL_LOG_FILE")]
    pub log_file: Option<PathBuf>,

    /// Bearer token required for *TCP* RPCs. Clients pass it via
    /// `OSDL_AUTH_TOKEN` (the CLI sets `authorization: Bearer <token>`
    /// on each request). UDS clients skip this — filesystem perms are
    /// the auth there. Required when the TCP listener binds to a
    /// non-loopback address; optional on loopback.
    #[arg(long, env = "OSDL_AUTH_TOKEN", hide_env_values = true)]
    pub auth_token: Option<String>,
}

/// Synchronous entrypoint called from `main`. Handles `--detach` *before*
/// building the tokio runtime — forking after tokio has registered fds
/// hands the child a broken epoll/kqueue.
pub fn main_entrypoint(mut args: ServeArgs) -> anyhow::Result<()> {
    #[cfg(unix)]
    let should_detach = args.detach;
    #[cfg(not(unix))]
    let should_detach = false;

    // Daemonized children chdir to `/` (standard daemon hygiene — don't
    // pin the user's CWD), so any relative path passed on the command
    // line stops resolving after the fork. Canonicalize them up-front
    // while we're still in the parent's CWD.
    if should_detach {
        absolutize_path_args(&mut args)?;
    }

    if should_detach {
        // Resolve paths in the parent so we can show the user the log
        // file location *before* exiting. After the daemonize call the
        // child is on its own.
        let paths = Paths::discover().map_err(|e| anyhow!("paths: {e}"))?;
        let log_path = resolve_log_path(&args, &paths)?;

        #[cfg(unix)]
        run_detached(args, log_path)?;
        Ok(())
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .init();
        run_foreground(args)
    }
}

/// Canonicalize all path-bearing CLI args so they survive the daemon's
/// chdir(`/`). We don't require the file to exist for `--data-dir` /
/// `--log-file` (they may be created), only resolve them relative to
/// the current CWD.
fn absolutize_path_args(args: &mut ServeArgs) -> anyhow::Result<()> {
    fn require_existing(p: &Path) -> anyhow::Result<PathBuf> {
        std::fs::canonicalize(p).with_context(|| format!("resolve {}", p.display()))
    }
    fn make_absolute(p: &Path) -> anyhow::Result<PathBuf> {
        if p.is_absolute() {
            Ok(p.to_path_buf())
        } else {
            let cwd = std::env::current_dir().context("read current dir")?;
            Ok(cwd.join(p))
        }
    }

    if let Some(p) = args.config.as_deref() {
        args.config = Some(require_existing(p)?);
    }
    if let Some(p) = args.registry.as_deref() {
        args.registry = Some(require_existing(p)?);
    }
    if let Some(p) = args.data_dir.as_deref() {
        args.data_dir = Some(make_absolute(p)?);
    }
    if let Some(p) = args.log_file.as_deref() {
        args.log_file = Some(make_absolute(p)?);
    }
    if let Some(s) = args.socket.as_deref() {
        // Skip non-path values like "disabled".
        if s != "disabled" && s != "none" && s != "off" {
            let abs = make_absolute(Path::new(s))?;
            args.socket = Some(abs.display().to_string());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn run_detached(args: ServeArgs, log_path: PathBuf) -> anyhow::Result<()> {
    use daemonize::{Daemonize, Outcome};
    use std::fs::OpenOptions;

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create log dir {}", parent.display()))?;
    }

    // Open the log file in the parent so a permission error surfaces
    // *before* the fork, not silently in the child.
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open log file {}", log_path.display()))?;
    let stderr = stdout.try_clone().context("clone log fd")?;

    // We deliberately do NOT use daemonize's pid_file: the engine's own
    // lockfile already records the post-fork PID, and racing two PID
    // sources is a recipe for stale-state confusion.
    let daemon = Daemonize::new()
        .working_directory("/")
        .umask(0o027)
        .stdout(stdout)
        .stderr(stderr);

    match daemon.execute() {
        Outcome::Parent(Ok(_)) => {
            // Print the human-readable handoff to the *original* terminal.
            // The child inherits the redirected stdio and won't see this.
            println!(
                "osdl: started instance '{}' in background; logs → {}",
                args.instance,
                log_path.display(),
            );
            println!("      stop with: osdl --instance {} stop", args.instance);
            Ok(())
        }
        Outcome::Parent(Err(e)) => Err(anyhow!("daemonize: {e}")),
        Outcome::Child(Err(e)) => {
            // The child failed mid-fork; surface to the log file then
            // exit non-zero so launchd-style supervisors notice.
            eprintln!("osdl: daemonize child failed: {e}");
            std::process::exit(1);
        }
        Outcome::Child(Ok(_)) => {
            // We're now the daemon. Set up logging *after* the fork,
            // because env_logger captures the current stderr fd at init
            // time — initialising before would log into the parent's
            // terminal.
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .init();
            log::info!("daemon child started; instance={}", args.instance);
            // Now that we're the daemon, run the same code path as
            // foreground mode under a fresh tokio runtime.
            run_foreground(args)
        }
    }
}

fn run_foreground(args: ServeArgs) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(run(args))
}

fn resolve_log_path(args: &ServeArgs, paths: &Paths) -> anyhow::Result<PathBuf> {
    if let Some(p) = &args.log_file {
        return Ok(p.clone());
    }
    let dir: &Path = args.data_dir.as_deref().unwrap_or(paths.state_dir.as_path());
    Ok(dir.join(format!("{}.log", args.instance)))
}

pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    let paths = Paths::discover().map_err(|e| anyhow!("paths: {e}"))?;

    // UDS is Unix-only. On Windows we can't bind one even if the user
    // asks for it, so we always set socket_path = None and fall back to
    // a TCP loopback default if --listen wasn't passed.
    #[cfg(unix)]
    let socket_path = match args.socket.as_deref() {
        Some("disabled") | Some("none") | Some("off") => None,
        Some(s) => Some(PathBuf::from(s)),
        None => Some(paths.default_socket_path(&args.instance)),
    };
    #[cfg(not(unix))]
    let socket_path: Option<PathBuf> = None;
    #[cfg(not(unix))]
    if let Some(s) = args.socket.as_deref() {
        if !matches!(s, "disabled" | "none" | "off") {
            log::warn!(
                "--socket={s} ignored on this platform (Unix domain sockets are not supported); \
                 using TCP loopback instead"
            );
        }
    }

    // On Windows we don't have UDS to fall back to, so default to TCP
    // loopback with a kernel-assigned port. The chosen port is recorded
    // in the lockfile, so clients can still discover us by --instance.
    #[cfg(not(unix))]
    let listen_addr = args.listen.or_else(|| {
        Some(SocketAddr::from(([127, 0, 0, 1], 0)))
    });
    #[cfg(unix)]
    let listen_addr = args.listen;

    if socket_path.is_none() && listen_addr.is_none() {
        return Err(anyhow!(
            "no listener configured: pass --listen ADDR or remove --socket=disabled"
        ));
    }

    // Refuse non-loopback TCP without an auth token *before* we boot the
    // broker / engine / event store — failing fast on a misconfiguration
    // is the whole point of this guard. (osdl_server::serve also rejects
    // it, but only after we've already started the heavy machinery.)
    if let Some(addr) = listen_addr {
        if args.auth_token.is_none() && !addr.ip().is_loopback() {
            return Err(anyhow!(
                "TCP listener {addr} is non-loopback but no --auth-token is configured. \
                 Set --auth-token / OSDL_AUTH_TOKEN, or bind to 127.0.0.1 / ::1 / UDS only."
            ));
        }
    }

    let config = build_config(&args)?;

    // Start broker + mDNS only when MQTT is enabled in the config.
    let _broker = config
        .mqtt
        .as_ref()
        .map(|c| EmbeddedBroker::start(c.port).map_err(|e| anyhow!("broker: {e}")))
        .transpose()?;
    let _mdns = config
        .mqtt
        .as_ref()
        .map(|c| MdnsAdvertiser::start(c.port).map_err(|e| anyhow!("mdns: {e}")))
        .transpose()?;

    if config.mqtt.is_some() {
        // Give the broker a moment to bind before the engine connects.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let data_dir = args.data_dir.clone().unwrap_or_else(|| paths.state_dir.clone());
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("create data dir {}", data_dir.display()))?;
    let db_path = data_dir.join(format!("{}.db", args.instance));
    let store = EventStore::open(&db_path).map_err(|e| anyhow!("event store {}: {}", db_path.display(), e))?;

    let adapters: Vec<Box<dyn osdl_core::adapter::ProtocolAdapter>> = vec![Box::new(
        UniLabOsAdapter::new(DriverRegistry::with_builtins()),
    )];
    let mut engine = OsdlEngine::new(config, adapters).with_store(store);
    let handle = engine.handle();

    // Engine runs in a dedicated task. We stop it explicitly after the
    // gRPC server returns so the shutdown order is: client → server → engine.
    let engine_task = tokio::spawn(async move { engine.run().await });

    let serve_cfg = ServeConfig {
        instance: args.instance.clone(),
        listen: ListenConfig {
            socket_path,
            tcp_addr: listen_addr,
        },
        lock_dir: paths.lock_dir(),
        auth_token: args.auth_token.clone(),
    };

    log::info!(
        "OpenSDL server starting (instance={}, db={}, lock_dir={})",
        args.instance,
        db_path.display(),
        paths.lock_dir().display()
    );

    let serve_result = osdl_server::serve(handle.clone(), serve_cfg).await;

    // Always tell the engine to stop on the way out — `serve` returning
    // doesn't currently propagate to the engine.
    handle.request_stop();
    if let Err(e) = engine_task.await {
        log::warn!("engine task failed to join: {e}");
    }

    serve_result.map_err(|e| anyhow!("serve: {e}"))
}

fn build_config(args: &ServeArgs) -> anyhow::Result<OsdlConfig> {
    let mut cfg = if let Some(path) = &args.config {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        serde_yaml::from_str::<OsdlConfig>(&raw)
            .with_context(|| format!("parse config {}", path.display()))?
    } else {
        // No config: ship a sensible default — MQTT broker on, unilabos
        // adapter, optional ESP-NOW dongle.
        OsdlConfig {
            mqtt: Some(MqttConfig::default()),
            adapters: vec![AdapterConfig {
                adapter_type: "unilabos".into(),
                registry_path: None,
            }],
            ..Default::default()
        }
    };

    // CLI/env-var overrides take precedence over YAML so the same recipe
    // config can be shared across machines that have different dongle
    // ports or registry paths.
    if let Some(reg) = &args.registry {
        let reg_str = reg.display().to_string();
        if cfg.adapters.is_empty() {
            cfg.adapters.push(AdapterConfig {
                adapter_type: "unilabos".into(),
                registry_path: Some(reg_str),
            });
        } else {
            for a in &mut cfg.adapters {
                a.registry_path = Some(reg_str.clone());
            }
        }
    } else {
        // Final fallback for adapters with no registry configured — keeps
        // the historical default behavior when running from the workspace
        // root with no flags.
        for a in &mut cfg.adapters {
            if a.registry_path.is_none() {
                a.registry_path = Some("registry/unilabos".to_string());
            }
        }
    }

    if let Some(port) = &args.dongle_port {
        if !port.is_empty() {
            // Replace any YAML-configured dongles: `--dongle-port` is the
            // authoritative answer for "which serial device is plugged in
            // *right now*", and shipping multiple at once would just
            // confuse the engine.
            cfg.espnow_dongles = vec![EspNowDongleConfig {
                port: port.clone(),
                baud_rate: args.dongle_baud,
            }];
        }
    }

    Ok(cfg)
}
