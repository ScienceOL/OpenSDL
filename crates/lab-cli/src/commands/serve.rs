//! `lab serve` — boot the engine + broker + mDNS, expose gRPC over UDS/TCP.
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
use lab_cli::runtime::{run_engine, EngineArgs, EngineMode};
use osdl_core::path_expand;
use osdl_server::{paths::Paths, ListenConfig, ServeConfig};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum AuthMode {
    Local,
    PublicKey,
    Static,
    Account,
}

#[derive(Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub engine: EngineArgs,

    /// Logical instance name. Determines the lockfile, default socket path,
    /// and default db filename.
    #[arg(long, env = "OSDL_INSTANCE", default_value = "default")]
    pub instance: String,

    /// Laboratory display name if there is no saved profile. Defaults to the host name.
    #[arg(long, env = "OSDL_LAB_NAME")]
    pub name: Option<String>,

    /// Where to bind the Unix domain socket. Default:
    /// `<runtime_dir>/<instance>.sock`. Set `--socket=disabled` to opt
    /// out of UDS entirely (then a TCP listener must be set).
    #[arg(long, env = "OSDL_SOCKET")]
    pub socket: Option<String>,

    /// Bind a TCP gRPC listener. Off by default — UDS only.
    #[arg(long, env = "OSDL_LISTEN")]
    pub listen: Option<SocketAddr>,

    /// Run in the background as a Unix daemon. Stdout/stderr are
    /// redirected to `--log-file`. The current process forks twice,
    /// detaches from the controlling terminal, and exits — leaving the
    /// daemon to run independently of your shell session. Use
    /// `lab stop --instance NAME` to terminate it.
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

    /// Token required by --auth static. Prefer the private environment.
    /// Unix sockets use filesystem permissions.
    #[arg(long, env = "OSDL_AUTH_TOKEN", hide_env_values = true)]
    pub auth_token: Option<String>,

    /// Explicit listener authority. Account uses the binding saved by `lab login`.
    #[arg(long, value_enum, default_value = "local")]
    pub auth: AuthMode,

    /// Treat a supervisor's static token as private local authority on an
    /// explicitly configured loopback TCP listener.
    #[arg(long, env = "OSDL_LOCAL_CAPABILITY")]
    pub local_capability: bool,
    #[arg(long, default_value="stable", value_parser=["dev","beta","rc","stable"])]
    pub channel: String,
}

/// Synchronous entrypoint called from `main`. Handles `--detach` *before*
/// building the tokio runtime — forking after tokio has registered fds
/// hands the child a broken epoll/kqueue.
pub fn main_entrypoint(mut args: ServeArgs) -> anyhow::Result<()> {
    #[cfg(unix)]
    let should_detach = args.detach;
    #[cfg(not(unix))]
    let should_detach = false;

    // Env-var expansion runs in *every* mode so users can write
    // `--config $LAB_DIR/recipe.yaml` from any shell (bash, fish, launchd,
    // Electron's child_process.spawn — none of them necessarily expand the
    // same surface). After expansion the path may still be relative — that
    // case is handled per-mode below.
    expand_env_in_path_args(&mut args);

    // Daemonized children chdir to `/` (standard daemon hygiene — don't
    // pin the user's CWD), so any relative path passed on the command
    // line stops resolving after the fork. Canonicalize them up-front
    // while we're still in the parent's CWD.
    if should_detach {
        absolutize_path_args(&mut args)?;
    }

    #[cfg(unix)]
    if should_detach {
        // Resolve paths in the parent so we can show the user the log
        // file location *before* exiting. After the daemonize call the
        // child is on its own.
        let paths = Paths::discover().map_err(|e| anyhow!("paths: {e}"))?;
        let log_path = resolve_log_path(&args, &paths)?;

        return run_detached(args, log_path);
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    run_foreground(args)
}

/// Expand `~` / `$VAR` / `${VAR}` in every path-shaped CLI arg in place.
/// Doesn't touch path *relativity* — that's handled in `absolutize_path_args`
/// or, for non-detach foreground runs, by std's normal CWD-relative open.
fn expand_env_in_path_args(args: &mut ServeArgs) {
    fn expand_pathbuf(p: &Path) -> PathBuf {
        PathBuf::from(path_expand::expand_vars(&p.to_string_lossy()))
    }
    args.engine.expand_paths();
    if let Some(p) = args.log_file.as_deref() {
        args.log_file = Some(expand_pathbuf(p));
    }
    if let Some(s) = args.socket.as_deref() {
        if !matches!(s, "disabled" | "none" | "off") {
            let expanded = path_expand::expand_vars(s);
            if expanded != s {
                args.socket = Some(expanded);
            }
        }
    }
}

/// Canonicalize all path-bearing CLI args so they survive the daemon's
/// chdir(`/`). We don't require the file to exist for `--data-dir` /
/// `--log-file` (they may be created), only resolve them relative to
/// the current CWD. Env vars are assumed to have already been expanded
/// by `expand_env_in_path_args`.
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

    if let Some(p) = args.engine.config.as_deref() {
        args.engine.config = Some(require_existing(p)?);
    }
    if let Some(p) = args.engine.registry.as_deref() {
        args.engine.registry = Some(require_existing(p)?);
    }
    if let Some(p) = args.engine.data_dir.as_deref() {
        args.engine.data_dir = Some(make_absolute(p)?);
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
                "lab: started instance '{}' in background; logs → {}",
                args.instance,
                log_path.display(),
            );
            println!("      stop with: lab --instance {} stop", args.instance);
            Ok(())
        }
        Outcome::Parent(Err(e)) => Err(anyhow!("daemonize: {e}")),
        Outcome::Child(Err(e)) => {
            // The child failed mid-fork; surface to the log file then
            // exit non-zero so launchd-style supervisors notice.
            eprintln!("lab: daemonize child failed: {e}");
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

#[cfg(unix)]
fn resolve_log_path(args: &ServeArgs, paths: &Paths) -> anyhow::Result<PathBuf> {
    if let Some(p) = &args.log_file {
        return Ok(p.clone());
    }
    let dir: &Path = args
        .engine
        .data_dir
        .as_deref()
        .unwrap_or(paths.state_dir.as_path());
    Ok(dir.join(format!("{}.log", args.instance)))
}

pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    anyhow::ensure!(
        osdl_auth::valid_instance_name(&args.instance),
        "instance must be 1–128 letters, digits, '-' or '_'"
    );
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
    let listen_addr = args
        .listen
        .or_else(|| Some(SocketAddr::from(([127, 0, 0, 1], 0))));
    #[cfg(unix)]
    let listen_addr = args.listen;

    if socket_path.is_none() && listen_addr.is_none() {
        return Err(anyhow!(
            "no listener configured: pass --listen ADDR or remove --socket=disabled"
        ));
    }

    anyhow::ensure!(
        !args.local_capability || (matches!(args.auth, AuthMode::Static) && args.listen.is_some()),
        "--local-capability requires --auth static and an explicit loopback --listen address"
    );

    // Resolve and validate authentication before starting the broker or hardware.
    // The server library enforces the same listener policy for non-CLI callers.
    let auth = match args.auth {
        AuthMode::Account => {
            anyhow::ensure!(
                args.auth_token.is_none(),
                "--auth account cannot be combined with --auth-token / OSDL_AUTH_TOKEN"
            );
            osdl_server::auth::ListenerAuth::Account(osdl_server::account::AccountAuth::new(
                osdl_server::account::Binding::load(&paths.config_dir, &args.instance)?,
            )?)
        }
        AuthMode::Local => {
            anyhow::ensure!(
                args.auth_token.is_none(),
                "local mode generates a private capability; use --auth static for process-supplied tokens"
            );
            osdl_server::auth::ListenerAuth::Local
        }
        AuthMode::PublicKey => {
            anyhow::ensure!(
                args.auth_token.is_none(),
                "public-key mode cannot use a bearer token"
            );
            let address = listen_addr
                .ok_or_else(|| anyhow!("public-key requires --listen TAILNET_IP:PORT"))?;
            let endpoint = osdl_auth::normalize_endpoint(&format!("https://{address}"))?;
            let data = args.engine.data_dir.as_ref().unwrap_or(&paths.state_dir);
            std::fs::create_dir_all(data)?;
            let channel = match args.channel.as_str() {
                "dev" => osdl_auth::Channel::Dev,
                "beta" => osdl_auth::Channel::Beta,
                "rc" => osdl_auth::Channel::Rc,
                _ => osdl_auth::Channel::Stable,
            };
            osdl_server::auth::ListenerAuth::PublicKey(
                osdl_server::public_key::PublicKeyAuth::open(
                    &std::path::absolute(data.join(format!("{}-public-key", args.instance)))?,
                    &args.instance,
                    channel,
                    &endpoint,
                )?,
            )
        }
        AuthMode::Static => match &args.auth_token {
            Some(token) if args.local_capability => {
                osdl_server::auth::ListenerAuth::LocalCapability(token.clone())
            }
            Some(token) => osdl_server::auth::ListenerAuth::Bearer(token.clone()),
            None => return Err(anyhow!("--auth static requires OSDL_AUTH_TOKEN")),
        },
    };
    auth.validate_listener(&args.instance, listen_addr)
        .map_err(anyhow::Error::msg)?;

    let advertise_mqtt = !matches!(args.auth, AuthMode::Local);
    let serve_cfg = ServeConfig {
        instance: args.instance.clone(),
        listen: ListenConfig {
            socket_path,
            tcp_addr: listen_addr,
        },
        lock_dir: paths.lock_dir(),
        auth,
    };
    run_engine(
        args.engine,
        &args.instance,
        paths.state_dir,
        EngineMode::Standalone { advertise_mqtt },
        |handle| async {
            osdl_server::serve_with_lab_name(handle, serve_cfg, args.name.as_deref())
                .await
                .map_err(Into::into)
        },
    )
    .await
}
