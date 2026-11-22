use std::future::Future;

use futures_util::{stream, StreamExt};
use osdl_proto::v1 as pb;
use prost::Message;
use tonic::{transport::Channel, Status};

use crate::{
    config::Id,
    directory::{Worker, PAGE_SIZE},
};

use super::{identity, Gateway};

pub(super) type WorkerClient = pb::osdl_client::OsdlClient<Channel>;
const MAX_INVENTORY_BYTES: usize = 4 * 1024 * 1024;
const MAX_GATEWAY_WORKERS: usize = 128;

impl Gateway {
    pub(super) fn client_for(&self, worker: &Worker) -> Result<WorkerClient, Status> {
        self.control.workers.channel(worker).map(WorkerClient::new)
    }

    async fn inventory<T, F, A>(&self, fetch: F) -> Result<Vec<(Id, Option<T>)>, Status>
    where
        T: Message + Send,
        F: Fn(WorkerClient, Id) -> A + Sync,
        A: Future<Output = Result<T, Status>> + Send,
    {
        let _permit = self.fanout_permit()?;
        let workers = self.worker_snapshot().await?;
        let mut inventory = Vec::with_capacity(workers.len());
        let mut bytes = 0;
        let mut responses = stream::iter(workers.into_iter().map(|worker| {
            let fetch = &fetch;
            async move {
                let result = match self.client_for(&worker) {
                    Ok(client) => fetch(client, worker.id.clone()).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = &result {
                    log::debug!(
                        "worker {} inventory unavailable: {}",
                        worker.id,
                        error.code()
                    );
                }
                (worker.id, result.ok())
            }
        }))
        .buffer_unordered(PAGE_SIZE);
        while let Some((worker, result)) = responses.next().await {
            if let Some(response) = &result {
                bytes += response.encoded_len();
                enforce_size(bytes)?;
            }
            inventory.push((worker, result));
        }
        Ok(inventory)
    }

    pub(super) async fn worker_snapshot(&self) -> Result<Vec<Worker>, Status> {
        self.control.directory.snapshot(MAX_GATEWAY_WORKERS).await
    }

    pub(super) async fn nodes(&self) -> Result<Vec<pb::Node>, Status> {
        let inventory = self
            .inventory(|mut client, worker| async move {
                let mut response =
                    crate::control::deadline(client.list_nodes(pb::ListNodesRequest {}))
                        .await?
                        .into_inner();
                for node in &mut response.nodes {
                    identity::node(&worker, node);
                }
                Ok(response)
            })
            .await?;
        if !inventory.is_empty() && inventory.iter().all(|(_, response)| response.is_none()) {
            return Err(Status::unavailable("no workers available"));
        }
        Ok(inventory
            .into_iter()
            .filter_map(|(_, response)| response)
            .flat_map(|response| response.nodes)
            .collect())
    }

    pub(super) async fn devices(
        &self,
        req: pb::ListDevicesRequest,
    ) -> Result<pb::ListDevicesResponse, Status> {
        let inventory = self
            .inventory(|mut client, worker| {
                let req = req.clone();
                async move {
                    let mut response = crate::control::deadline(client.list_devices(req))
                        .await?
                        .into_inner();
                    for device in &mut response.devices {
                        identity::device(&worker, device);
                    }
                    response
                        .workers
                        .retain(|status| status.id == worker.as_str());
                    response.workers.truncate(1);
                    Ok(response)
                }
            })
            .await?;
        let mut response = pb::ListDevicesResponse::default();
        for (worker, result) in inventory {
            let available = result.is_some();
            let mut status = match result {
                Some(mut result) => {
                    response.devices.append(&mut result.devices);
                    result.workers.pop().unwrap_or_default()
                }
                None => pb::WorkerStatus::default(),
            };
            status.id = worker.to_string();
            status.available = available;
            response.workers.push(status);
        }
        response
            .workers
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        enforce_size(response.encoded_len())?;
        Ok(response)
    }
}

fn enforce_size(bytes: usize) -> Result<(), Status> {
    if bytes > MAX_INVENTORY_BYTES {
        return Err(Status::resource_exhausted(
            "cluster inventory exceeds the OpenSDL response limit",
        ));
    }
    Ok(())
}
