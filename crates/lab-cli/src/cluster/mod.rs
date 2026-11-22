pub mod config;
mod probes;
pub mod tls;

use std::path::Path;

use osdl_core::EngineHandle;
use osdl_server::cluster::WorkerListener;
use tokio::task::JoinSet;

pub struct Worker {
    listener: WorkerListener,
    probes: Vec<probes::DeviceProbe>,
}

impl Worker {
    pub async fn bind(path: &Path) -> anyhow::Result<Self> {
        let config: config::WorkerConfig = config::read(path)?;
        let listen = config::worker_address(config.listen)?;
        probes::validate(&config.read_only_devices)?;
        log::info!(
            "starting worker {} in cluster {}",
            config.worker,
            config.cluster
        );
        let tls = config.incoming.load(path)?;
        let listener = tls
            .worker_listener(config.worker.to_string(), listen)
            .await?;
        Ok(Self {
            listener,
            probes: config.read_only_devices,
        })
    }

    pub fn name(&self) -> &str {
        self.listener.worker_name()
    }

    pub async fn start(
        self,
        engine: &EngineHandle,
    ) -> anyhow::Result<(WorkerListener, JoinSet<()>)> {
        let tasks = probes::start(self.probes, engine).await?;
        Ok((self.listener, tasks))
    }
}
