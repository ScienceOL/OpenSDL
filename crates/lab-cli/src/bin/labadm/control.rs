use std::time::Duration;

use futures_util::{stream, StreamExt};
use osdl_proto::{cluster::v1 as api, v1 as osdl};
use tonic::{transport::Channel, Request, Response, Status, Streaming};

use crate::{
    config::Id,
    directory::{Directory, Worker, PAGE_SIZE},
    routes::Workers,
    tls::RPC_TIMEOUT,
};

// An inventory page can contain eight worker responses, each bounded by tonic.
pub const INVENTORY_RESPONSE_LIMIT: usize = PAGE_SIZE * 4 * 1024 * 1024 + 4096;

#[derive(Clone)]
pub struct Control {
    pub cluster: Id,
    pub directory: Directory,
    pub workers: Workers,
}

impl Control {
    async fn client(&self, worker: &str) -> Result<osdl::osdl_client::OsdlClient<Channel>, Status> {
        let route = self.directory.worker(&parse_id(worker)?).await?;
        Ok(osdl::osdl_client::OsdlClient::new(
            self.workers.channel(&route)?,
        ))
    }

    async fn inventory(&self, worker: Worker) -> api::WorkerInventory {
        let result = async {
            let mut client =
                api::work_plane_client::WorkPlaneClient::new(self.workers.channel(&worker)?);
            deadline(client.inventory(api::WorkerInventoryRequest {})).await
        }
        .await;
        let (devices, unavailable) = match result {
            Ok(response) => (response.into_inner().devices, None),
            Err(error) => (
                Vec::new(),
                Some(format!("{}: {}", error.code(), error.message())),
            ),
        };
        api::WorkerInventory {
            worker: worker.id.to_string(),
            devices,
            unavailable,
        }
    }
}

#[tonic::async_trait]
impl api::control_plane_server::ControlPlane for Control {
    type StreamEventsStream = Streaming<osdl::Event>;
    type PublishMediaStream = Streaming<osdl::PublishMediaStatus>;

    async fn ready(
        &self,
        _: Request<api::ReadyRequest>,
    ) -> Result<Response<api::ReadyResponse>, Status> {
        self.directory.ready().await?;
        Ok(Response::new(api::ReadyResponse {
            cluster: self.cluster.to_string(),
        }))
    }

    async fn list_workers(
        &self,
        request: Request<api::WorkerPage>,
    ) -> Result<Response<api::WorkerList>, Status> {
        let page = self
            .directory
            .page(parse_after(&request.into_inner().after)?)
            .await?;
        Ok(Response::new(api::WorkerList {
            workers: page
                .workers
                .into_iter()
                .map(|worker| worker.id.to_string())
                .collect(),
            next_after: page.next_after,
        }))
    }

    async fn list_devices(
        &self,
        request: Request<api::InventoryRequest>,
    ) -> Result<Response<api::Inventory>, Status> {
        let request = request.into_inner();
        let (workers, next_after) = if request.worker.is_empty() {
            let page = self.directory.page(parse_after(&request.after)?).await?;
            (page.workers, page.next_after)
        } else {
            if !request.after.is_empty() {
                return Err(Status::invalid_argument(
                    "worker and after are mutually exclusive",
                ));
            }
            (
                vec![self.directory.worker(&parse_id(&request.worker)?).await?],
                String::new(),
            )
        };
        // Each page bounds both fan-out and the size of the response.
        let workers = stream::iter(workers.into_iter().map(|worker| self.inventory(worker)))
            .buffered(PAGE_SIZE)
            .collect()
            .await;
        Ok(Response::new(api::Inventory {
            workers,
            next_after,
        }))
    }

    async fn get_device(
        &self,
        request: Request<api::DeviceRequest>,
    ) -> Result<Response<osdl::Device>, Status> {
        let request = request.into_inner();
        let mut worker = self.client(&request.worker).await?;
        deadline(worker.get_device(osdl::GetDeviceRequest {
            device_id: request.device_id,
        }))
        .await
    }

    async fn send_command(
        &self,
        request: Request<api::CommandRequest>,
    ) -> Result<Response<osdl::CommandResult>, Status> {
        let request = request.into_inner();
        let command = request
            .command
            .ok_or_else(|| Status::invalid_argument("command is required"))?;
        let mut worker = self.client(&request.worker).await?;
        let result = deadline(worker.send_command(command)).await;
        // Once dispatched, a transport failure cannot prove whether an action
        // reached the instrument. This path never tries a second request.
        result.map_err(|error| match error.code() {
            tonic::Code::Unavailable
            | tonic::Code::DeadlineExceeded
            | tonic::Code::Cancelled
            | tonic::Code::Unknown => Status::unknown(
                "command outcome unknown. Check the instrument state before resubmitting",
            ),
            _ => error,
        })
    }

    async fn stream_events(
        &self,
        request: Request<api::EventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let request = request.into_inner();
        let mut worker = self.client(&request.worker).await?;
        deadline(worker.stream_events(request.filter.unwrap_or_default())).await
    }

    async fn read_snapshot(
        &self,
        request: Request<api::SnapshotRequest>,
    ) -> Result<Response<osdl::ReadSnapshotResponse>, Status> {
        let request = request.into_inner();
        let snapshot = request
            .snapshot
            .ok_or_else(|| Status::invalid_argument("snapshot is required"))?;
        let mut worker = self.client(&request.worker).await?;
        deadline(worker.read_snapshot(snapshot)).await
    }

    async fn publish_media(
        &self,
        request: Request<api::MediaRequest>,
    ) -> Result<Response<Self::PublishMediaStream>, Status> {
        let request = request.into_inner();
        let media = request
            .media
            .ok_or_else(|| Status::invalid_argument("media request is required"))?;
        let mut worker = self.client(&request.worker).await?;
        // The owning service can wait up to 15 seconds for FFmpeg startup.
        forward(Duration::from_secs(18), worker.publish_media(media)).await
    }
}

fn parse_id(name: &str) -> Result<Id, Status> {
    name.parse()
        .map_err(|_| Status::invalid_argument("invalid worker ID"))
}

fn parse_after(name: &str) -> Result<Option<Id>, Status> {
    if name.is_empty() {
        Ok(None)
    } else {
        parse_id(name).map(Some)
    }
}

pub(super) async fn deadline<T>(
    rpc: impl std::future::Future<Output = Result<Response<T>, Status>>,
) -> Result<Response<T>, Status> {
    forward(RPC_TIMEOUT, rpc).await
}

async fn forward<T>(
    timeout: Duration,
    rpc: impl std::future::Future<Output = Result<Response<T>, Status>>,
) -> Result<Response<T>, Status> {
    let response = tokio::time::timeout(timeout, rpc)
        .await
        .map_err(|_| Status::deadline_exceeded("worker request timed out"))??;
    Ok(Response::new(response.into_inner()))
}
