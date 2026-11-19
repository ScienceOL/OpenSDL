use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use osdl_proto::v1::osdl_client::OsdlClient;
use tonic::{
    transport::{Channel, Endpoint},
    Status,
};

use crate::{
    config::Id,
    directory::Worker,
    tls::{Tls, RPC_TIMEOUT},
};

const CHANNEL_CAPACITY: usize = 128;

#[derive(Clone)]
pub struct Workers(Arc<Routes>);

type Channels = HashMap<(Id, SocketAddr), Channel>;

struct Routes {
    cluster: Id,
    tls: Tls,
    channels: Mutex<Channels>,
}

impl Workers {
    pub fn new(cluster: Id, tls: Tls) -> Self {
        Self(Arc::new(Routes {
            cluster,
            tls,
            channels: Mutex::new(HashMap::new()),
        }))
    }

    pub fn channel(&self, worker: &Worker) -> Result<Channel, Status> {
        let key = (worker.id.clone(), worker.address);
        let mut channels = self
            .0
            .channels
            .lock()
            .map_err(|_| Status::internal("worker connection cache unavailable"))?;
        if let Some(channel) = channels.get(&key) {
            return Ok(channel.clone());
        }

        let endpoint = Endpoint::from_shared(format!("https://{}", worker.address))
            .map_err(|_| Status::internal("invalid worker endpoint"))?;
        let channel = self
            .0
            .tls
            .endpoint(endpoint, Some(worker.id.worker_tls_name(&self.0.cluster)))
            .map_err(|err| {
                log::error!("worker {} TLS setup failed: {err}", worker.id);
                Status::internal("worker TLS configuration unavailable")
            })?
            .connect_lazy();
        if channels.len() == CHANNEL_CAPACITY {
            if let Some(old) = channels.keys().next().cloned() {
                channels.remove(&old);
            }
        }
        channels.insert(key, channel.clone());
        Ok(channel)
    }

    pub async fn verify(&self, worker: &Worker) -> anyhow::Result<()> {
        let response = tokio::time::timeout(
            RPC_TIMEOUT,
            OsdlClient::new(self.channel(worker)?).status(osdl_proto::v1::StatusRequest {}),
        )
        .await??
        .into_inner();
        anyhow::ensure!(
            response.instance == worker.id.as_str(),
            "worker answered with a different engine instance"
        );
        Ok(())
    }
}
