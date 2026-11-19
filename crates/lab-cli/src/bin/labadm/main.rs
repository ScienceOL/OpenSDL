mod config;
mod control;
mod directory;
mod etcd;
mod operator;
mod routes;
mod tls;
mod worker;

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lab_cli::runtime::EngineArgs;
use osdl_proto::cluster::v1::control_plane_server::ControlPlaneServer;
use tokio::{net::TcpListener, signal};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

use config::{ControlConfig, Id, WorkerConfig};
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let Cli {
        cluster_config,
        command,
    } = Cli::parse();
    let path = &cluster_config;
    match command {
        Command::Join(mut engine) => {
            let config: WorkerConfig = config::read(path)?;
            let tls = config.incoming.load(path)?;
            engine.expand_paths();
            log::info!(
                "starting worker {} in cluster {}",
                config.worker,
                config.cluster
            );
            worker::serve(config, tls, engine).await
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
            log::info!("control plane listening on {}", listener.local_addr()?);
            let serving = server
                .add_service(
                    ControlPlaneServer::new(control)
                        .max_encoding_message_size(control::INVENTORY_RESPONSE_LIMIT),
                )
                .serve_with_incoming(TcpListenerStream::new(listener));
            tokio::select! {
                result = serving => result?,
                result = signal::ctrl_c() => result?,
            }
        }
        Command::Join(_) | Command::Operator(_) => unreachable!("handled by main"),
    }
    Ok(())
}
