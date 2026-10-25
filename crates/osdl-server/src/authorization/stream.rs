//! Authorization owns the upstream independently of HTTP/2 response-body polling.
use super::lease::{expiry_status, Resource};
use osdl_auth::now;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt};
use tonic::Status;

type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

pub(crate) async fn expires(lease: Option<&Resource>) -> Status {
    match lease {
        Some(lease) => {
            lease.expired().await;
            lease.expiry_status()
        }
        None => std::future::pending::<Status>().await,
    }
}

pub(crate) fn guard_stream<T: Send + 'static>(
    mut stream: ResponseStream<T>,
    lease: Option<Resource>,
) -> ResponseStream<T> {
    let Some(lease) = lease else { return stream };
    let deadline = lease.deadline();
    let grant = lease.is_grant();
    let (sender, mut receiver) = mpsc::channel(1);
    // One lifecycle task owns both the publisher/subscription and its lease.
    // Reserving before reading preserves backpressure with at most one queued item.
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = lease.expired() => {},
            _ = sender.closed() => {},
            _ = async {
                loop {
                    let Ok(permit) = sender.reserve().await else { break };
                    let Some(item) = stream.next().await else { break };
                    let terminal = item.is_err();
                    permit.send(item);
                    if terminal { break; }
                }
            } => {},
        }
        // Release resources before closing the handoff and waking the response body.
        drop(stream);
        drop(lease);
    });
    Box::pin(async_stream::stream! {
        loop {
            let item = receiver.recv().await;
            // A response stalled past expiry must discard its queued item, including
            // when the upstream ended earlier. This observer never retains the lease.
            let expiry = *deadline.borrow();
            if expiry <= now() {
                yield Err(expiry_status(grant, expiry));
                break;
            }
            match item {
                Some(item) => yield item,
                None => break,
            }
        }
    })
}
