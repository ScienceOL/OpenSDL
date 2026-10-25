//! Authorization owns the upstream independently of HTTP/2 response-body polling.
use super::{identity::now, lease::Lease, AuthError};
use std::{pin::Pin, sync::Arc};
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt};
use tonic::Status;

type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

pub(crate) async fn expires(lease: Option<&Arc<Lease>>) {
    match lease {
        Some(lease) => lease.expired().await,
        None => std::future::pending::<()>().await,
    }
}

pub(crate) fn guard_stream<T: Send + 'static>(
    mut stream: ResponseStream<T>,
    lease: Option<Arc<Lease>>,
) -> ResponseStream<T> {
    let Some(lease) = lease else { return stream };
    let deadline = lease.deadline();
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
            if *deadline.borrow() <= now() {
                yield Err(AuthError::Expired.status());
                break;
            }
            match item {
                Some(item) => yield item,
                None => break,
            }
        }
    })
}
