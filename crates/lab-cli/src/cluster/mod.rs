pub mod config;
pub mod tls;

use std::path::Path;

use osdl_server::cluster::WorkerListener;

pub async fn worker_listener(path: &Path) -> anyhow::Result<WorkerListener> {
    let config: config::WorkerConfig = config::read(path)?;
    let listen = config::worker_address(config.listen)?;
    log::info!(
        "starting worker {} in cluster {}",
        config.worker,
        config.cluster
    );
    let tls = config.incoming.load(path)?;
    tls.worker_listener(config.worker.to_string(), listen).await
}
