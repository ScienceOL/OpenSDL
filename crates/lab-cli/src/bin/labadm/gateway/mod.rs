mod events;
mod identity;
mod inventory;
mod wait;

use std::{pin::Pin, sync::Arc};

use futures_util::{Stream, StreamExt};
use osdl_proto::{
    cluster::v1::{
        control_plane_server::ControlPlane, CommandRequest, DeviceRequest, MediaRequest,
        SnapshotRequest,
    },
    v1::{self as pb, osdl_server::Osdl},
};
use osdl_server::{convert::now_ts, SERVER_VERSION};
use tokio::sync::Semaphore;
use tonic::{Code, Request, Response, Status};

use crate::{control::Control, directory::PAGE_SIZE};

const EVENT_SUBSCRIPTION_LIMIT: usize = 8;
const FANOUT_REQUEST_LIMIT: usize = 8;

type RpcStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

#[derive(Clone)]
pub struct Gateway {
    control: Arc<Control>,
    instance: Arc<str>,
    started_at: prost_types::Timestamp,
    event_dials: Arc<Semaphore>,
    event_subscriptions: Arc<Semaphore>,
    fanout_requests: Arc<Semaphore>,
}

impl Gateway {
    pub fn new(control: Control, instance: String) -> Self {
        Self {
            control: Arc::new(control),
            instance: instance.into(),
            started_at: now_ts(),
            event_dials: Arc::new(Semaphore::new(PAGE_SIZE)),
            event_subscriptions: Arc::new(Semaphore::new(EVENT_SUBSCRIPTION_LIMIT)),
            fanout_requests: Arc::new(Semaphore::new(FANOUT_REQUEST_LIMIT)),
        }
    }

    fn fanout_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, Status> {
        self.fanout_requests
            .try_acquire()
            .map_err(|_| Status::resource_exhausted("cluster fan-out requests are busy"))
    }
}

#[tonic::async_trait]
impl Osdl for Gateway {
    type StreamEventsStream = RpcStream<pb::Event>;
    type PublishMediaStream = RpcStream<pb::PublishMediaStatus>;

    async fn status(
        &self,
        _: Request<pb::StatusRequest>,
    ) -> Result<Response<pb::StatusResponse>, Status> {
        self.control.directory.ready().await?;
        Ok(Response::new(pb::StatusResponse {
            version: SERVER_VERSION.into(),
            instance: self.instance.to_string(),
            pid: std::process::id(),
            started_at: Some(self.started_at),
            engine: Some(pb::EngineStatus {
                state: pb::engine_status::State::Connected as i32,
                // Status checks directory readiness. Counts describe no local engine.
                broker: "cluster".into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
    }

    async fn list_nodes(
        &self,
        _: Request<pb::ListNodesRequest>,
    ) -> Result<Response<pb::ListNodesResponse>, Status> {
        Ok(Response::new(pb::ListNodesResponse {
            nodes: self.nodes().await?,
        }))
    }

    async fn list_devices(
        &self,
        req: Request<pb::ListDevicesRequest>,
    ) -> Result<Response<pb::ListDevicesResponse>, Status> {
        let devices = self.devices(req.into_inner()).await?;
        Ok(Response::new(pb::ListDevicesResponse { devices }))
    }

    async fn get_device(
        &self,
        req: Request<pb::GetDeviceRequest>,
    ) -> Result<Response<pb::Device>, Status> {
        let req = req.into_inner();
        let (worker, local_id) = identity::split(&req.device_id)?;
        let mut device = self
            .control
            .get_device(Request::new(DeviceRequest {
                worker: worker.to_string(),
                device_id: local_id.into(),
            }))
            .await
            .map_err(|err| safe_error(err, "device lookup failed"))?
            .into_inner();
        identity::device(&worker, &mut device);
        Ok(Response::new(device))
    }

    async fn wait_for_device(
        &self,
        req: Request<pb::WaitForDeviceRequest>,
    ) -> Result<Response<pb::Device>, Status> {
        Ok(Response::new(self.wait(req.into_inner()).await?))
    }

    async fn send_command(
        &self,
        req: Request<pb::SendCommandRequest>,
    ) -> Result<Response<pb::CommandResult>, Status> {
        let mut command = req.into_inner();
        let (worker, local_id) = identity::split(&command.device_id)?;
        command.device_id = local_id.into();
        let mut result = self
            .control
            .send_command(Request::new(CommandRequest {
                worker: worker.to_string(),
                command: Some(command),
            }))
            .await
            .map_err(command_error)?
            .into_inner();
        identity::command_result(&worker, &mut result);
        Ok(Response::new(result))
    }

    async fn stream_events(
        &self,
        req: Request<pb::StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        Ok(Response::new(self.events(req.into_inner()).await?))
    }

    async fn publish_media(
        &self,
        req: Request<pb::PublishMediaRequest>,
    ) -> Result<Response<Self::PublishMediaStream>, Status> {
        let mut media = req.into_inner();
        let (worker, local_id) = identity::split(&media.source_id)?;
        media.source_id = local_id.into();
        let stream = self
            .control
            .publish_media(Request::new(MediaRequest {
                worker: worker.to_string(),
                media: Some(media),
            }))
            .await
            .map_err(|err| safe_error(err, "worker media publisher unavailable"))?
            .into_inner()
            .map(|item| item.map_err(|err| safe_error(err, "worker media publisher stopped")));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn read_snapshot(
        &self,
        req: Request<pb::ReadSnapshotRequest>,
    ) -> Result<Response<pb::ReadSnapshotResponse>, Status> {
        let req = req.into_inner();
        let (worker, local_path) = identity::split(&req.path)?;
        let path = identity::snapshot_path(local_path)?.into();
        self.control
            .read_snapshot(Request::new(SnapshotRequest {
                worker: worker.to_string(),
                snapshot: Some(pb::ReadSnapshotRequest { path }),
            }))
            .await
            .map_err(|err| safe_error(err, "worker snapshot unavailable"))
    }

    async fn shutdown(
        &self,
        _: Request<pb::ShutdownRequest>,
    ) -> Result<Response<pb::ShutdownResponse>, Status> {
        Err(Status::permission_denied(
            "gateway shutdown requires local administration",
        ))
    }
}

fn safe_error(error: Status, message: &'static str) -> Status {
    // Worker status details may contain local transport addresses or filenames.
    Status::new(error.code(), message)
}

fn command_error(error: Status) -> Status {
    match error.code() {
        Code::InvalidArgument | Code::NotFound | Code::PermissionDenied | Code::Unauthenticated => {
            safe_error(error, "command rejected")
        }
        Code::Unavailable => safe_error(error, "cluster route unavailable before dispatch"),
        // Worker I/O or acknowledgement failure doesn't establish that the
        // instrument did not begin the action.
        _ => Status::unknown(
            "command outcome unknown. Check the instrument state before resubmitting",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_errors_do_not_claim_that_a_dispatched_action_failed() {
        let transport = command_error(Status::failed_precondition("private transport address"));
        assert_eq!(transport.code(), Code::Unknown);
        assert!(!transport.message().contains("private"));
        let before_dispatch = command_error(Status::unavailable("worker directory down"));
        assert_eq!(before_dispatch.code(), Code::Unavailable);
        let rejected = command_error(Status::not_found("worker missing"));
        assert_eq!(rejected.code(), Code::NotFound);
    }
}
