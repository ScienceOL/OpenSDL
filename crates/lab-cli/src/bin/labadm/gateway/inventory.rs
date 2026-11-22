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

    pub(super) async fn inventory<T, F, A>(&self, fetch: F) -> Result<Vec<T>, Status>
    where
        T: Message + Send,
        F: Fn(WorkerClient, Id) -> A + Sync,
        A: Future<Output = Result<Vec<T>, Status>> + Send,
    {
        let _permit = self.fanout_permit()?;
        let workers = self.worker_snapshot().await?;
        let has_workers = !workers.is_empty();
        let mut inventory = Vec::new();
        let mut bytes = 0;
        let mut reachable = false;
        let mut responses = stream::iter(workers.into_iter().map(|worker| {
            let fetch = &fetch;
            async move {
                let result = fetch(self.client_for(&worker)?, worker.id.clone()).await;
                if let Err(error) = &result {
                    log::debug!(
                        "worker {} inventory unavailable: {}",
                        worker.id,
                        error.code()
                    );
                }
                result
            }
        }))
        .buffer_unordered(PAGE_SIZE);
        while let Some(result) = responses.next().await {
            if let Ok(values) = result {
                reachable = true;
                for value in values {
                    let size = value.encoded_len();
                    bytes += 1 + prost::length_delimiter_len(size) + size;
                    if bytes > MAX_INVENTORY_BYTES {
                        return Err(Status::resource_exhausted(
                            "cluster inventory exceeds the OpenSDL response limit",
                        ));
                    }
                    inventory.push(value);
                }
            }
        }
        if has_workers && !reachable {
            return Err(Status::unavailable("no workers available"));
        }
        Ok(inventory)
    }

    pub(super) async fn worker_snapshot(&self) -> Result<Vec<Worker>, Status> {
        self.control.directory.snapshot(MAX_GATEWAY_WORKERS).await
    }

    pub(super) async fn nodes(&self) -> Result<Vec<pb::Node>, Status> {
        self.inventory(|mut client, worker| async move {
            let mut nodes = crate::control::deadline(client.list_nodes(pb::ListNodesRequest {}))
                .await?
                .into_inner()
                .nodes;
            for node in &mut nodes {
                identity::node(&worker, node);
            }
            Ok(nodes)
        })
        .await
    }

    pub(super) async fn devices(
        &self,
        req: pb::ListDevicesRequest,
    ) -> Result<Vec<pb::Device>, Status> {
        self.inventory(|mut client, worker| {
            let req = req.clone();
            async move {
                let mut devices = crate::control::deadline(client.list_devices(req))
                    .await?
                    .into_inner()
                    .devices;
                for device in &mut devices {
                    identity::device(&worker, device);
                }
                Ok(devices)
            }
        })
        .await
    }
}
