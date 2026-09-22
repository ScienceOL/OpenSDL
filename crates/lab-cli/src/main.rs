//! `lab` — the OpenSDL command-line tool.
//!
//! Subcommands split into two families:
//!   * `serve` boots the engine + gRPC server in this process.
//!   * everything else is a gRPC client that talks to a running server.

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

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authenticate to SciLaxy and inspect online lab devices.
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
    /// Ask the running server to shut down.
    Stop,
    /// Validate a portable OpenSDL virtual asset directory.
    Validate(commands::validate::ValidateArgs),
    /// Pack a portable asset into a deterministic OCI image layout.
    Pack(commands::pack::PackArgs),
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
    let result: anyhow::Result<()> = match cli.command {
        Command::Serve(args) => commands::serve::main_entrypoint(args),
        Command::Validate(args) => commands::validate::run(args),
        Command::Pack(args) => commands::pack::run(args),
        Command::Inspect(args) => commands::inspect::run(args),
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
                    Command::Status => commands::status::run(opts).await,
                    Command::Device { cmd } => commands::device::run(cmd, opts).await,
                    Command::Send(args) => commands::send::run(args, opts).await,
                    Command::Events(args) => commands::events::run(args, opts).await,
                    Command::Stop => commands::stop::run(opts).await,
                    Command::Push(args) => commands::push::run(args).await,
                    Command::Pull(args) => commands::pull::run(args).await,
                    Command::Api { cmd } => commands::api::run(cmd).await,
                    Command::Validate(_) | Command::Pack(_) | Command::Inspect(_) => unreachable!(),
                }
            })
        }
    };

    if let Err(e) = result {
        eprintln!("lab: {e:#}");
        std::process::exit(1);
    }
}
