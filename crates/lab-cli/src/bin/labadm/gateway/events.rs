use std::time::Duration;

use futures_util::{stream, StreamExt};
use osdl_proto::v1 as pb;
use tonic::{Code, Status, Streaming};

use crate::{config::Id, control::deadline, directory::Worker};

use super::{identity, Gateway, RpcStream};

const INITIAL_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(30);

impl Gateway {
    pub(super) async fn events(
        &self,
        filter: pb::StreamEventsRequest,
    ) -> Result<RpcStream<pb::Event>, Status> {
        let routes = self.stream_routes().await?;
        let streams = routes.into_iter().map(|route| {
            let feed = Feed {
                gateway: self.clone(),
                worker: route.id.clone(),
                initial_route: Some(route),
                stream: None,
                filter: filter.clone(),
                backoff: Duration::ZERO,
            };
            Box::pin(stream::unfold(feed, Feed::next))
        });
        Ok(Box::pin(stream::select_all(streams).map(Ok)))
    }
}

struct Feed {
    gateway: Gateway,
    worker: Id,
    initial_route: Option<Worker>,
    stream: Option<Streaming<pb::Event>>,
    filter: pb::StreamEventsRequest,
    backoff: Duration,
}

impl Feed {
    async fn next(mut self) -> Option<(pb::Event, Self)> {
        loop {
            if let Some(stream) = &mut self.stream {
                match stream.message().await {
                    Ok(Some(mut event)) => {
                        self.backoff = Duration::ZERO;
                        project(&self.worker, &mut event);
                        return Some((event, self));
                    }
                    Ok(None) => {}
                    Err(error) => log::debug!(
                        "worker {} event stream ended: {}",
                        self.worker,
                        error.code()
                    ),
                }
                self.stream = None;
            } else {
                if !self.backoff.is_zero() {
                    tokio::time::sleep(self.backoff).await;
                }
                match self.connect().await {
                    Ok(stream) => {
                        self.stream = Some(stream);
                        continue;
                    }
                    Err(error) if error.code() == Code::NotFound => return None,
                    Err(error) => log::debug!(
                        "worker {} event stream unavailable: {}",
                        self.worker,
                        error.code()
                    ),
                }
            }
            let first_failure = self.backoff.is_zero();
            self.backoff = (self.backoff * 2).clamp(INITIAL_RETRY, MAX_RETRY);
            if first_failure && wants_worker_unavailable(&self.filter) {
                let event = pb::Event {
                    timestamp: Some(osdl_server::convert::now_ts()),
                    kind: Some(pb::event::Kind::WorkerUnavailable(
                        pb::WorkerUnavailableEvent {
                            worker: self.worker.to_string(),
                        },
                    )),
                };
                return Some((event, self));
            }
        }
    }

    async fn connect(&mut self) -> Result<Streaming<pb::Event>, Status> {
        let _dial = self
            .gateway
            .event_dials
            .acquire()
            .await
            .map_err(|_| Status::unavailable("worker event stream unavailable"))?;
        let route = match self.initial_route.take() {
            Some(route) => route,
            None => self.gateway.control.directory.worker(&self.worker).await?,
        };
        let mut client = self.gateway.client_for(&route)?;
        Ok(deadline(client.stream_events(self.filter.clone()))
            .await?
            .into_inner())
    }
}

fn wants_worker_unavailable(filter: &pb::StreamEventsRequest) -> bool {
    filter.kinds.is_empty() || filter.kinds.iter().any(|kind| kind == "worker_unavailable")
}

fn project(worker: &Id, event: &mut pb::Event) {
    use pb::event::Kind;
    match &mut event.kind {
        Some(Kind::DeviceOnline(ev)) => {
            if let Some(device) = &mut ev.device {
                identity::device(worker, device);
            }
        }
        Some(Kind::DeviceOffline(ev)) => identity::qualify(worker, &mut ev.device_id),
        Some(Kind::DeviceStatus(ev)) => {
            identity::properties(worker, &ev.device_id, &mut ev.properties);
            identity::qualify(worker, &mut ev.device_id);
        }
        Some(Kind::CommandResult(ev)) => {
            if let Some(result) = &mut ev.result {
                identity::command_result(worker, result);
            }
        }
        Some(Kind::UnknownNode(ev)) => identity::qualify(worker, &mut ev.node_id),
        Some(Kind::MediaSourceOnline(ev)) => {
            identity::qualify(worker, &mut ev.id);
            // Playback uses the authenticated publisher on the owner.
            ev.endpoints.clear();
        }
        Some(Kind::MediaGatewayDown(ev)) => {
            ev.source_namespace = Some(worker.to_string());
            ev.reason = "worker media gateway down".into();
        }
        Some(Kind::WorkerUnavailable(ev)) => ev.worker = worker.to_string(),
        Some(Kind::Lagged(_)) | None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_events_expose_only_the_routed_source() {
        let worker = "scienceol".parse().unwrap();
        let mut event = pb::Event {
            kind: Some(pb::event::Kind::MediaSourceOnline(
                pb::MediaSourceOnlineEvent {
                    id: "cam131".into(),
                    endpoints: vec![pb::MediaEndpoint {
                        url: "rtsp://private-address".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        project(&worker, &mut event);
        let Some(pb::event::Kind::MediaSourceOnline(online)) = event.kind else {
            panic!("event changed kind");
        };
        assert_eq!(online.id, "scienceol:cam131");
        assert!(online.endpoints.is_empty());
    }

    #[test]
    fn gateway_down_is_confined_to_its_worker_namespace() {
        let worker = "scienceol".parse().unwrap();
        let mut event = pb::Event {
            kind: Some(pb::event::Kind::MediaGatewayDown(
                pb::MediaGatewayDownEvent {
                    reason: "private runtime details".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        project(&worker, &mut event);
        let Some(pb::event::Kind::MediaGatewayDown(down)) = event.kind else {
            panic!("event changed kind");
        };
        assert_eq!(down.source_namespace.as_deref(), Some("scienceol"));
        assert!(!down.reason.contains("private"));
    }
}
