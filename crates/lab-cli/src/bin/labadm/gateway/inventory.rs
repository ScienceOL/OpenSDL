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
const MAX_STREAM_WORKERS: usize = 128;

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
        let mut inventory = Vec::new();
        let mut bytes = 0;
        let mut after = None;
        let mut reachable = false;
        let mut has_workers = false;
        loop {
            let page = self.control.directory.page(after).await?;
            has_workers |= !page.workers.is_empty();
            let mut responses = stream::iter(page.workers.into_iter().map(|worker| {
                let fetch = &fetch;
                async move { fetch(self.client_for(&worker)?, worker.id).await }
            }))
            .buffer_unordered(PAGE_SIZE);
            while let Some(result) = responses.next().await {
                match result {
                    Ok(values) => {
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
                    Err(error) => {
                        log::debug!("worker inventory unavailable: {}", error.code());
                    }
                }
            }

            if page.next_after.is_empty() {
                break;
            }
            after = Some(parse_cursor(&page.next_after)?);
        }
        if has_workers && !reachable {
            return Err(Status::unavailable("no workers available"));
        }
        Ok(inventory)
    }

    pub(super) async fn stream_routes(&self) -> Result<Vec<Worker>, Status> {
        let mut workers = Vec::new();
        let mut after = None;
        loop {
            let page = self.control.directory.page(after).await?;
            workers.extend(page.workers);
            if workers.len() > MAX_STREAM_WORKERS {
                return Err(Status::resource_exhausted(
                    "cluster has too many workers for a single OpenSDL stream",
                ));
            }
            if page.next_after.is_empty() {
                return Ok(workers);
            }
            after = Some(parse_cursor(&page.next_after)?);
        }
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

fn parse_cursor(cursor: &str) -> Result<Id, Status> {
    cursor
        .parse()
        .map_err(|_| Status::data_loss("invalid worker page cursor"))
}
