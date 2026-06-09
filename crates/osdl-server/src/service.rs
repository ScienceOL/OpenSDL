//! tonic service implementation backed by an `EngineHandle`.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use osdl_core::protocol::DeviceCommand;
use osdl_core::{EngineHandle, OsdlEvent};
use osdl_proto::v1 as pb;
use tokio::sync::{broadcast, Notify};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::convert::{
    command_result_to_pb, device_to_pb, event_to_pb, hashmap_to_struct, json_to_struct,
    lagged_event, node_to_pb, now_ts, status_to_pb, struct_to_json_map,
};

/// Shared identity the gRPC layer needs at request-handling time.
#[derive(Debug, Clone)]
pub struct ServerIdentity {
    pub instance: String,
    pub version: String,
    pub pid: u32,
    pub started_at: prost_types::Timestamp,
    pub socket_path: Option<String>,
    pub listen_addr: Option<String>,
}

#[derive(Clone)]
pub struct OsdlService {
    engine: EngineHandle,
    identity: ServerIdentity,
    /// Fires when the server is shutting down. `StreamEvents` watches
    /// this so live streams release their RPC slot and tonic's graceful
    /// shutdown can finish — without it, an open events stream wedges
    /// the daemon forever (the broadcast Sender never closes because
    /// the service still holds it).
    shutdown: Arc<Notify>,
}

impl OsdlService {
    pub fn new(engine: EngineHandle, identity: ServerIdentity) -> Self {
        Self::with_shutdown(engine, identity, Arc::new(Notify::new()))
    }

    pub fn with_shutdown(
        engine: EngineHandle,
        identity: ServerIdentity,
        shutdown: Arc<Notify>,
    ) -> Self {
        Self {
            engine,
            identity,
            shutdown,
        }
    }
}

#[tonic::async_trait]
impl pb::osdl_server::Osdl for OsdlService {
    async fn status(
        &self,
        _req: Request<pb::StatusRequest>,
    ) -> Result<Response<pb::StatusResponse>, Status> {
        let engine = status_to_pb(&self.engine.status());
        Ok(Response::new(pb::StatusResponse {
            version: self.identity.version.clone(),
            instance: self.identity.instance.clone(),
            pid: self.identity.pid,
            socket_path: self.identity.socket_path.clone(),
            listen_addr: self.identity.listen_addr.clone(),
            engine: Some(engine),
            started_at: Some(self.identity.started_at.clone()),
        }))
    }

    async fn list_nodes(
        &self,
        _req: Request<pb::ListNodesRequest>,
    ) -> Result<Response<pb::ListNodesResponse>, Status> {
        let nodes = self
            .engine
            .list_nodes()
            .await
            .iter()
            .map(node_to_pb)
            .collect();
        Ok(Response::new(pb::ListNodesResponse { nodes }))
    }

    async fn list_devices(
        &self,
        req: Request<pb::ListDevicesRequest>,
    ) -> Result<Response<pb::ListDevicesResponse>, Status> {
        let filter = req.into_inner();
        let devices = self
            .engine
            .list_devices()
            .await
            .into_iter()
            .filter(|d| {
                filter.adapter.as_deref().is_none_or(|a| d.adapter == a)
                    && filter.device_type.as_deref().is_none_or(|t| d.device_type == t)
                    && filter
                        .role
                        .as_deref()
                        .is_none_or(|r| d.role.as_deref() == Some(r))
            })
            .map(|d| device_to_pb(&d))
            .collect();
        Ok(Response::new(pb::ListDevicesResponse { devices }))
    }

    async fn get_device(
        &self,
        req: Request<pb::GetDeviceRequest>,
    ) -> Result<Response<pb::Device>, Status> {
        let device_id = req.into_inner().device_id;
        match self.engine.get_device(&device_id).await {
            Some(d) => Ok(Response::new(device_to_pb(&d))),
            None => Err(Status::not_found(format!("unknown device: {device_id}"))),
        }
    }

    async fn wait_for_device(
        &self,
        req: Request<pb::WaitForDeviceRequest>,
    ) -> Result<Response<pb::Device>, Status> {
        let req = req.into_inner();
        let timeout = if req.timeout_ms == 0 {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(req.timeout_ms as u64)
        };

        // Subscribe BEFORE checking state — otherwise the device could come
        // online between our snapshot and the subscribe and we'd miss it.
        let mut events = self.engine.subscribe_events();

        // Snapshot match.
        let matcher: Box<dyn Fn(&osdl_core::Device) -> bool + Send + Sync> = match req.selector {
            Some(pb::wait_for_device_request::Selector::DeviceId(id)) => {
                Box::new(move |d| d.id == id)
            }
            Some(pb::wait_for_device_request::Selector::DeviceType(t)) => {
                Box::new(move |d| d.device_type == t)
            }
            Some(pb::wait_for_device_request::Selector::Role(r)) => {
                Box::new(move |d| d.role.as_deref() == Some(r.as_str()))
            }
            None => {
                return Err(Status::invalid_argument(
                    "wait_for_device requires a selector (device_id, device_type, or role)",
                ));
            }
        };

        if let Some(d) = self.engine.list_devices().await.into_iter().find(|d| matcher(d)) {
            return Ok(Response::new(device_to_pb(&d)));
        }

        // Block on DeviceOnline events until match or deadline.
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Status::deadline_exceeded(
                    "no matching device appeared in time",
                ));
            }
            let recv = tokio::time::timeout(remaining, events.recv()).await;
            match recv {
                Ok(Ok(OsdlEvent::DeviceOnline(d))) if matcher(&d) => {
                    return Ok(Response::new(device_to_pb(&d)));
                }
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    // The matching DeviceOnline event might have been in
                    // the dropped window. Re-snapshot before resuming —
                    // otherwise we'd block until timeout despite the
                    // device already being registered.
                    if let Some(d) = self
                        .engine
                        .list_devices()
                        .await
                        .into_iter()
                        .find(|d| matcher(d))
                    {
                        return Ok(Response::new(device_to_pb(&d)));
                    }
                    continue;
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(Status::aborted("event stream closed"));
                }
                Err(_) => {
                    return Err(Status::deadline_exceeded(
                        "no matching device appeared in time",
                    ));
                }
            }
        }
    }

    async fn send_command(
        &self,
        req: Request<pb::SendCommandRequest>,
    ) -> Result<Response<pb::CommandResult>, Status> {
        let r = req.into_inner();
        let command_id = if r.command_id.is_empty() {
            // Cheap monotonic-ish id. The server reflects it back so the
            // client can correlate to the eventual CommandResult event.
            format!("srv-{}", now_ms())
        } else {
            r.command_id
        };

        let params = r
            .params
            .as_ref()
            .map(|s| serde_json::Value::Object(struct_to_json_map(s)))
            .unwrap_or(serde_json::Value::Null);

        let cmd = DeviceCommand {
            command_id: command_id.clone(),
            device_id: r.device_id,
            action: r.action,
            params,
        };

        let result = self
            .engine
            .send_command(cmd)
            .await
            .map_err(Status::failed_precondition)?;

        Ok(Response::new(command_result_to_pb(&result)))
    }

    type StreamEventsStream =
        Pin<Box<dyn Stream<Item = Result<pb::Event, Status>> + Send + 'static>>;

    async fn stream_events(
        &self,
        req: Request<pb::StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let kinds = req.into_inner().kinds;
        // Subscribe BEFORE reading the snapshot so we don't lose any
        // event fired between the snapshot read and the loop start.
        // The snapshot may overlap with a live event still in the
        // broadcast buffer; clients are expected to dedupe by id (the
        // runner already does this via its `media_sources` cache).
        let mut rx = self.engine.subscribe_events();
        let shutdown = self.shutdown.clone();

        // Replay the current media-source snapshot so a subscriber
        // that connects after engine startup learns about cameras —
        // the live `MediaSourceOnline` event is only emitted once at
        // mediamtx start and there's no ListMediaSources RPC for
        // late-comers to reconcile against.
        let media_snapshot = self.engine.list_media_sources().await;

        let stream = async_stream::stream! {
            // Synthesize a `MediaSourceOnline` event per snapshot entry
            // and pipe it through the same `event_to_pb` used for live
            // events so the wire shape stays in sync with the canonical
            // converter.
            for src in media_snapshot {
                let synthetic = osdl_core::OsdlEvent::MediaSourceOnline {
                    id: src.id,
                    description: src.description,
                    endpoints: src.endpoints,
                };
                let pb_event = event_to_pb(&synthetic);
                if !kinds.is_empty() && !event_kind_matches(&pb_event, &kinds) {
                    continue;
                }
                yield Ok(pb_event);
            }
            // Pre-allocate a `Notified` future so the select! loop can
            // be polled across iterations. `Notified` resets after each
            // wake; we re-arm it at the top of every iteration.
            loop {
                let notified = shutdown.notified();
                tokio::pin!(notified);
                tokio::select! {
                    biased;
                    _ = &mut notified => {
                        // Server is shutting down. Cleanly end the
                        // stream so tonic's graceful shutdown can
                        // finish draining this RPC.
                        break;
                    }
                    msg = rx.recv() => match msg {
                        Ok(ev) => {
                            let pb_event = event_to_pb(&ev);
                            if !kinds.is_empty() && !event_kind_matches(&pb_event, &kinds) {
                                continue;
                            }
                            yield Ok(pb_event);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            yield Ok(lagged_event(n));
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn shutdown(
        &self,
        _req: Request<pb::ShutdownRequest>,
    ) -> Result<Response<pb::ShutdownResponse>, Status> {
        self.engine.request_stop();
        Ok(Response::new(pb::ShutdownResponse {}))
    }
}

fn event_kind_matches(ev: &pb::Event, allowed: &[String]) -> bool {
    let name = match &ev.kind {
        Some(pb::event::Kind::DeviceOnline(_)) => "device_online",
        Some(pb::event::Kind::DeviceOffline(_)) => "device_offline",
        Some(pb::event::Kind::DeviceStatus(_)) => "device_status",
        Some(pb::event::Kind::CommandResult(_)) => "command_result",
        Some(pb::event::Kind::UnknownNode(_)) => "unknown_node",
        Some(pb::event::Kind::MediaSourceOnline(_)) => "media_source_online",
        Some(pb::event::Kind::MediaGatewayDown(_)) => "media_gateway_down",
        Some(pb::event::Kind::Lagged(_)) => "lagged",
        None => return false,
    };
    allowed.iter().any(|k| k == name)
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// Suppress unused import warnings for items only referenced inside other
// trait impls or future code.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = (now_ts(), hashmap_to_struct, json_to_struct);
}
