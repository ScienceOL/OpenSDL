use crate::authorization::{lease::Resource, stream::expires};
use crate::convert::device_to_pb;
use osdl_core::{EngineHandle, OsdlEvent};
use osdl_proto::v1 as pb;
use std::time::Duration;
use tokio::sync::broadcast;
use tonic::{Response, Status};

// The waiting future owns its subscription. select drops it before releasing the lease.
pub(super) async fn wait(
    engine: &EngineHandle,
    req: pb::WaitForDeviceRequest,
    mut events: broadcast::Receiver<OsdlEvent>,
    lease: Option<Resource>,
) -> Result<Response<pb::Device>, Status> {
    let waiting = async move {
        let timeout = if req.timeout_ms == 0 {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(req.timeout_ms as u64)
        };

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

        if let Some(d) = engine.list_devices().await.into_iter().find(|d| matcher(d)) {
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
                    if let Some(d) = engine.list_devices().await.into_iter().find(|d| matcher(d)) {
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
    };
    tokio::select! {
        biased;
        status = expires(lease.as_ref()) => Err(status),
        result = waiting => result,
    }
}

#[cfg(test)]
mod tests;
