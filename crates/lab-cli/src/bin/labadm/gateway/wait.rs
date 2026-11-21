use std::time::Duration;

use futures_util::{stream::FuturesUnordered, StreamExt};
use osdl_proto::v1::{self as pb, wait_for_device_request::Selector};
use tokio::time::{timeout_at, Instant};
use tonic::{Code, Status};

use super::{identity, safe_error, Gateway};

impl Gateway {
    pub(super) async fn wait(
        &self,
        mut req: pb::WaitForDeviceRequest,
    ) -> Result<pb::Device, Status> {
        let timeout_ms = match req.timeout_ms {
            0 => 30_000,
            ms => ms.min(30_000),
        };
        req.timeout_ms = timeout_ms;
        let until = Instant::now() + Duration::from_millis(timeout_ms.into());
        timeout_at(until, self.wait_on_workers(req))
            .await
            .map_err(|_| timed_out())?
    }

    async fn wait_on_workers(
        &self,
        mut req: pb::WaitForDeviceRequest,
    ) -> Result<pb::Device, Status> {
        match req.selector.as_mut() {
            Some(Selector::DeviceId(id)) => {
                let (worker, local_id) = identity::split(id)?;
                *id = local_id.into();
                let route = self.control.directory.worker(&worker).await?;
                let mut client = self.client_for(&route)?;
                let mut device = client
                    .wait_for_device(req)
                    .await
                    .map_err(|err| safe_error(err, "worker device wait failed"))?
                    .into_inner();
                identity::device(&worker, &mut device);
                Ok(device)
            }
            Some(Selector::DeviceType(_) | Selector::Role(_)) => {
                let _permit = self.fanout_permit()?;
                let mut calls = FuturesUnordered::new();
                for worker in self.worker_snapshot().await? {
                    let req = req.clone();
                    calls.push(async move {
                        let mut client = self.client_for(&worker)?;
                        let mut device = client.wait_for_device(req).await?.into_inner();
                        identity::device(&worker.id, &mut device);
                        Ok::<_, Status>(device)
                    });
                }
                let mut worker_waited = false;
                while let Some(response) = calls.next().await {
                    match response {
                        Ok(device) => return Ok(device),
                        Err(err) if err.code() == Code::DeadlineExceeded => worker_waited = true,
                        Err(_) => {}
                    }
                }
                if worker_waited {
                    Err(timed_out())
                } else {
                    Err(Status::unavailable("no workers available"))
                }
            }
            None => Err(Status::invalid_argument(
                "wait_for_device requires a selector",
            )),
        }
    }
}

fn timed_out() -> Status {
    Status::deadline_exceeded("no matching device appeared in time")
}
