mod control;
mod directory;
mod etcd;
mod gateway;
mod operator;
mod routes;

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lab_cli::cluster::{self, config, tls};
use lab_cli::runtime::{run_engine, EngineArgs, EngineMode};
use osdl_proto::cluster::v1::control_plane_server::ControlPlaneServer;
use osdl_server::account::{AccountAdminEndpoint, AccountAuth};
use osdl_server::MediaRelay;
use tokio::{net::TcpListener, signal};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

use config::{AccountGatewayConfig, ControlConfig, Id};
use control::Control;
use directory::{Directory, Worker};
use etcd::Etcd;
use routes::Workers;

#[derive(Parser)]
#[command(version, about = "Manage an OpenSDL laboratory cluster")]
struct Cli {
    /// Read this cluster, worker, or operator configuration.
    #[arg(long, global = true, default_value = "labadm.yaml")]
    cluster_config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the shared directory for this cluster.
    Init,
    /// Run a control-plane replica in the foreground.
    Serve,
    /// Run a work-plane node and its local OpenSDL engine.
    Join(EngineArgs),
    /// Admit the exact network address of a verified worker.
    Admit { worker: Id, address: SocketAddr },
    /// Remove a worker route from the shared directory.
    Forget { worker: Id },
    #[command(flatten)]
    Operator(operator::Command),
}

#[tokio::main]
async fn main() -> Result<()> {
    osdl_auth::crypto_provider();
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,opensdl::account_audit=info"),
    )
    .init();
    let Cli {
        cluster_config,
        command,
    } = Cli::parse();
    let path = &cluster_config;
    match command {
        Command::Join(mut engine) => {
            engine.expand_paths();
            let worker = cluster::Worker::bind(path).await?;
            let instance = worker.name().to_owned();
            let paths = osdl_server::Paths::discover().map_err(anyhow::Error::msg)?;
            run_engine(
                engine,
                &instance,
                paths.state_dir,
                EngineMode::WorkPlane {
                    advertise_mqtt: false,
                },
                |handle| async {
                    let (listener, _probes) = worker.start(&handle).await?;
                    listener.serve(handle).await.map_err(Into::into)
                },
            )
            .await
        }
        Command::Operator(command) => operator::run(command, path).await,
        command => manage(command, path).await,
    }
}

async fn manage(command: Command, path: &Path) -> Result<()> {
    let config: ControlConfig = config::read(path)?;
    let directory = Directory::new(
        Etcd::connect(
            config.directory.endpoints,
            &config.directory.tls.load(path)?,
        )?,
        &config.cluster,
    );
    match command {
        Command::Init => {
            directory
                .initialize()
                .await
                .context("initialize cluster directory")?;
            println!("initialized cluster {}", config.cluster);
        }
        Command::Forget { worker } => {
            directory.forget(&worker).await.context("forget worker")?;
            println!("removed route for {worker}");
        }
        Command::Admit { worker, address } => {
            let worker = Worker {
                id: worker,
                address: config::worker_address(address)?,
            };
            let clients = Workers::new(config.cluster, config.workers.load(path)?);
            clients
                .verify(&worker)
                .await
                .with_context(|| format!("verify worker {}", worker.id))?;
            directory.admit(&worker).await.context("admit worker")?;
            println!("admitted {} at {}", worker.id, worker.address);
        }
        Command::Serve => {
            directory.ready().await.context("check shared directory")?;
            let workers = Workers::new(config.cluster.clone(), config.workers.load(path)?);
            let control = Control {
                cluster: config.cluster,
                directory,
                workers,
            };
            let mut server = Server::builder().tls_config(config.incoming.load(path)?.server())?;
            let listener = TcpListener::bind(config.listen).await?;
            let gateway = prepare_account_gateway(config.account_gateway, &control).await?;
            log::info!("control plane listening on {}", listener.local_addr()?);
            let serving = server
                .add_service(
                    ControlPlaneServer::new(control)
                        .max_encoding_message_size(control::INVENTORY_RESPONSE_LIMIT),
                )
                .serve_with_incoming(TcpListenerStream::new(listener));
            tokio::select! {
                result = serving => result?,
                result = run_account_gateway(gateway) => result?,
                result = signal::ctrl_c() => result?,
            }
        }
        Command::Join(_) | Command::Operator(_) => unreachable!("handled by main"),
    }
    Ok(())
}

struct AccountGateway {
    listener: TcpListener,
    account: AccountAuth,
    administration: AccountAdminEndpoint,
    profile_dir: PathBuf,
    gateway: gateway::Gateway,
    media_relay: Option<MediaRelay>,
}

async fn prepare_account_gateway(
    config: Option<AccountGatewayConfig>,
    control: &Control,
) -> Result<Option<AccountGateway>> {
    let Some(config) = config else {
        return Ok(None);
    };
    let paths = osdl_server::Paths::discover().map_err(anyhow::Error::msg)?;
    let account =
        AccountAuth::open(&paths.config_dir, &config.instance).context("load gateway account")?;
    let administration = AccountAdminEndpoint::bind(&account, &paths)
        .await
        .context("bind local gateway administration")?;
    let media_relay = config
        .media_relay
        .as_deref()
        .map(str::parse)
        .transpose()
        .context("validate media relay")?;
    let listener = TcpListener::bind(config.listen)
        .await
        .context("bind account gateway")?;
    let gateway = gateway::Gateway::new(control.clone(), config.instance);
    Ok(Some(AccountGateway {
        listener,
        account,
        administration,
        profile_dir: paths.state_dir,
        gateway,
        media_relay,
    }))
}

async fn run_account_gateway(gateway: Option<AccountGateway>) -> Result<()> {
    let Some(AccountGateway {
        listener,
        account,
        administration,
        profile_dir,
        gateway,
        media_relay,
    }) = gateway
    else {
        return std::future::pending().await;
    };
    log::info!("account gateway listening on {}", listener.local_addr()?);
    tokio::select! {
        result = osdl_server::serve_account_endpoint(listener, account, gateway, media_relay, &profile_dir) => result.context("serve account gateway"),
        result = administration.serve() => result.context("serve local gateway administration"),
    }
}
