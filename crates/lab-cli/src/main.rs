//! `lab` — the OpenSDL command-line tool.
//!
//! Asset commands operate on the local resource store or OCI registries.
//! `serve` hosts the engine, while device commands use its gRPC API.

mod client;
mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "lab", version, about = "OpenSDL command-line interface")]
struct Cli {
    /// Connect to this endpoint instead of auto-discovering. Examples:
    /// `unix:/run/osdl/default.sock`, `http://lab-pi.local:50051`.
    #[arg(long, env = "OSDL_ENDPOINT", global = true)]
    endpoint: Option<String>,

    /// Resolve to this server instance via the lockfile dir. Use when
    /// multiple `lab serve` processes are running on the host.
    #[arg(long, env = "OSDL_INSTANCE", global = true)]
    instance: Option<String>,

    /// Bearer token attached to every TCP RPC. Required when the
    /// remote server has auth enabled. Ignored on UDS (filesystem
    /// perms are the auth there).
    #[arg(long, env = "OSDL_AUTH_TOKEN", global = true, hide_env_values = true)]
    auth_token: Option<String>,

    /// Local resource store (packages, catalog and managed instances).
    #[arg(long, env = "LAB_ASSET_CACHE", global = true)]
    cache_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Bind a laboratory instance through browser account approval.
    Login(commands::login::LoginArgs),
    /// Manage local digest-pinned runtime instances.
    Instance {
        #[command(subcommand)]
        cmd: commands::instance::InstanceCmd,
    },
    #[command(hide = true)]
    InstanceWorker {
        #[arg(long)]
        name: String,
        #[arg(long)]
        generation: i64,
    },
    /// Manage locally cached asset packages and references.
    Asset {
        #[command(subcommand)]
        cmd: commands::asset::AssetCmd,
    },
    /// Own the local content-addressed asset store.
    Daemon {
        #[command(subcommand)]
        cmd: commands::daemon::DaemonCmd,
    },
    /// Authenticate to LiyanLabs and inspect online lab devices.
    Api {
        #[command(subcommand)]
        cmd: commands::api::ApiCmd,
    },
    /// Boot the engine + gRPC server in this process.
    Serve(commands::serve::ServeArgs),
    /// Show server identity and engine status.
    Status,
    /// Inspect known devices.
    Device {
        #[command(subcommand)]
        cmd: commands::device::DeviceCmd,
    },
    /// Send a single command to a device.
    Send(commands::send::SendArgs),
    /// Stream engine events.
    Events(commands::events::EventsArgs),
    /// Run a digest-pinned kinematic asset as a managed local instance.
    ///
    /// This is the short form of `lab instance run`. The explicit name keeps
    /// scripts reproducible and lets several instances use one cache.
    Run {
        reference: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        offline: bool,
        #[command(flatten)]
        auth: commands::registry_auth::RegistryAuthArgs,
    },
    /// List managed local instances (the short form of `lab instance list`).
    Ps {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Start a retained managed instance.
    Start {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Stop an instance, or shut down the standalone server when no name is given.
    Stop {
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print retained logs for a managed instance.
    Logs {
        name: String,
        #[arg(long, default_value_t = 100)]
        tail: usize,
    },
    /// Remove a stopped managed instance.
    Rm { name: String },
    /// List cached asset packages (the short form of `lab asset list`).
    Images {
        #[arg(long)]
        json: bool,
    },
    /// Validate a portable OpenSDL virtual asset directory.
    Validate(commands::validate::ValidateArgs),
    /// Pack a portable asset into a deterministic OCI image layout.
    Pack(commands::pack::PackArgs),
    /// Build a portable asset into the local image store and optionally tag it.
    Build(commands::build::BuildArgs),
    /// Split selected OpenUSD prims into independently buildable child assets.
    Split(commands::split::SplitArgs),
    /// Inspect a portable asset directory or OCI image layout.
    Inspect(commands::inspect::InspectArgs),
    /// Push a packed virtual asset to an OCI registry.
    Push(commands::push::PushArgs),
    /// Pull a virtual asset into the local cache and optionally materialize it.
    Pull(commands::pull::PullArgs),
}

fn main() {
    let cli = Cli::parse();

    // `serve --detach` daemonizes the process *before* the tokio runtime
    // starts — forking after tokio has registered fds with epoll/kqueue
    // hands the child a broken reactor. So we route Serve through a
    // dedicated entrypoint that owns the daemonize → runtime sequence,
    // and keep `#[tokio::main]`-equivalent behavior for everything else.
    let cache_dir = cli.cache_dir;
    let result: anyhow::Result<()> = match cli.command {
        Command::Serve(args) => commands::serve::main_entrypoint(args),
        Command::Validate(args) => commands::validate::run(args),
        Command::Pack(args) => commands::pack::run(args),
        Command::Split(args) => commands::split::run(args),
        other => {
            // Client subcommands run a vanilla tokio runtime in the foreground.
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .init();
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("lab: failed to build tokio runtime: {e}");
                    std::process::exit(1);
                }
            };
            let opts = client::ClientOpts {
                endpoint: cli.endpoint,
                instance: cli.instance,
                auth_token: cli.auth_token,
            };
            rt.block_on(async move {
                match other {
                    Command::Serve(_) => unreachable!(),
                    Command::Login(args) => commands::login::run(args, opts.instance).await,
                    Command::Asset { cmd } => commands::asset::run(cmd, cache_dir).await,
                    Command::Daemon { cmd } => commands::daemon::run(cmd, cache_dir).await,
                    Command::Inspect(args) => commands::inspect::run(args, cache_dir).await,
                    Command::Status => commands::status::run(opts).await,
                    Command::Device { cmd } => commands::device::run(cmd, opts).await,
                    Command::Send(args) => commands::send::run(args, opts).await,
                    Command::Events(args) => commands::events::run(args, opts).await,
                    Command::Run {
                        reference,
                        name,
                        json,
                        offline,
                        auth,
                    } => {
                        commands::instance::run(
                            commands::instance::InstanceCmd::Run {
                                reference,
                                name,
                                json,
                                offline,
                                auth,
                            },
                            cache_dir,
                        )
                        .await
                    }
                    Command::Ps { all, json } => {
                        commands::instance::run(
                            commands::instance::InstanceCmd::List { all, json },
                            cache_dir,
                        )
                        .await
                    }
                    Command::Start { name, json } => {
                        commands::instance::run(
                            commands::instance::InstanceCmd::Start { name, json },
                            cache_dir,
                        )
                        .await
                    }
                    Command::Stop { name, json } => match name {
                        Some(name) => {
                            commands::instance::run(
                                commands::instance::InstanceCmd::Stop { name, json },
                                cache_dir,
                            )
                            .await
                        }
                        None if json => Err(anyhow::anyhow!(
                            "--json requires an instance name; use `lab stop` for the standalone server"
                        )),
                        None => commands::stop::run(opts).await,
                    },
                    Command::Logs { name, tail } => {
                        commands::instance::run(
                            commands::instance::InstanceCmd::Logs { name, tail },
                            cache_dir,
                        )
                        .await
                    }
                    Command::Rm { name } => {
                        commands::instance::run(
                            commands::instance::InstanceCmd::Remove { name },
                            cache_dir,
                        )
                        .await
                    }
                    Command::Images { json } => {
                        commands::asset::run(
                            commands::asset::AssetCmd::List { json },
                            cache_dir,
                        )
                        .await
                    }
                    Command::Push(args) => commands::push::run(args, cache_dir).await,
                    Command::Pull(args) => commands::pull::run(args, cache_dir).await,
                    Command::Build(args) => commands::build::run(args, cache_dir).await,
                    Command::Api { cmd } => commands::api::run(cmd).await,
                    Command::Instance { cmd } => commands::instance::run(cmd, cache_dir).await,
                    Command::InstanceWorker { name, generation } => {
                        osdl_server::managed::worker::run(
                            cache_dir.ok_or_else(|| {
                                anyhow::anyhow!("worker requires an explicit resource store")
                            })?,
                            name,
                            generation,
                        )
                        .await
                    }
                    Command::Validate(_)
                    | Command::Pack(_)
                    | Command::Split(_) => unreachable!(),
                }
            })
        }
    };

    if let Err(e) = result {
        eprintln!("lab: {e:#}");
        std::process::exit(1);
    }
}
