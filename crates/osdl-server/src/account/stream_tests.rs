use super::{lease::Leases, stream::guard_stream, Identity, Principal, Scope};
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};
use tokio::sync::oneshot;
use tokio_stream::Stream;
use tonic::Status;

struct ReadyStream {
    reads: Arc<AtomicUsize>,
    started: Option<oneshot::Sender<()>>,
    dropped: Option<oneshot::Sender<()>>,
}
impl Stream for ReadyStream {
    type Item = Result<u8, Status>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(started) = self.started.take() {
            let _ = started.send(());
        }
        Poll::Ready(Some(Ok(1)))
    }
}
impl Drop for ReadyStream {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}
#[tokio::test]
async fn bounded_handoff_and_unpolled_consumer_cancellation_release_all_owners() {
    let registry = Leases::default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut bodies = Vec::new();
    let mut completions = Vec::new();
    for _ in 0..128 {
        let identity = Identity {
            principal: Principal {
                device_id: uuid::Uuid::new_v4(),
                device_generation: 1,
                installation_id: uuid::Uuid::new_v4(),
            },
            scope: Scope::Read,
            expires_at: now + 300,
        };
        let reads = Arc::new(AtomicUsize::new(0));
        let (started, ready) = oneshot::channel();
        let (dropped, done) = oneshot::channel();
        let inner = Box::pin(ReadyStream {
            reads: reads.clone(),
            started: Some(started),
            dropped: Some(dropped),
        });
        bodies.push(guard_stream(inner, Some(registry.acquire(&identity))));
        completions.push(done);
        tokio::time::timeout(std::time::Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        // The ready upstream must stop at one queued item without body polling.
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }
    assert_eq!(registry.len(), 128);
    drop(bodies);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        for done in completions {
            done.await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(registry.len(), 0);
}

#[tokio::test]
async fn measure_bounded_stream_delivery_for_short_and_long_histories() {
    use tokio_stream::StreamExt;
    let registry = Leases::default();
    for count in [100, 10000] {
        for account in [false, true] {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let identity = Identity {
                principal: Principal {
                    device_id: uuid::Uuid::new_v4(),
                    device_generation: 1,
                    installation_id: uuid::Uuid::new_v4(),
                },
                scope: Scope::Read,
                expires_at: now + 300,
            };
            let lease = account.then(|| registry.acquire(&identity));
            let start = std::time::Instant::now();
            let mut stream = guard_stream(Box::pin(tokio_stream::iter((0..count).map(Ok))), lease);
            let mut received = 0;
            while let Some(item) = stream.next().await {
                assert_eq!(item.unwrap(), received);
                received += 1;
            }
            assert_eq!(received, count);
            assert_eq!(registry.len(), 0);
            println!("PERF bounded_stream account={account} items={count} elapsed_us={} retained_leases={}",start.elapsed().as_micros(),registry.len());
        }
    }
}
