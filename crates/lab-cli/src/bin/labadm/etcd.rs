use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::ensure;
use futures_util::{stream::FuturesUnordered, StreamExt};
use osdl_proto::etcd::{kv_client::KvClient, RangeRequest, RangeResponse, TxnRequest};
use tonic::{transport::Channel, Code, Request, Response, Status};

use crate::{config::https_endpoint, tls::Tls};

const DEADLINE: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Etcd(Arc<Pool>);

struct Pool {
    clients: Vec<KvClient<Channel>>,
    preferred: AtomicUsize,
}

impl Etcd {
    pub fn connect(endpoints: Vec<String>, tls: &Tls) -> anyhow::Result<Self> {
        ensure!(
            !endpoints.is_empty(),
            "configure at least one etcd endpoint"
        );
        let clients = endpoints
            .into_iter()
            .map(|address| {
                let channel = tls.endpoint(https_endpoint(address)?, None)?.connect_lazy();
                // Directory entries are small and every range has a limit.
                Ok(KvClient::new(channel).max_decoding_message_size(128 * 1024))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self(Arc::new(Pool {
            clients,
            preferred: AtomicUsize::new(0),
        })))
    }

    pub async fn range(&self, request: RangeRequest) -> Result<RangeResponse, Status> {
        self.call(move |mut client| {
            let request = timed_request(request.clone());
            async move { client.range(request).await }
        })
        .await
    }

    // All callers use compare-and-swap transactions. A retry can fail the
    // comparison after a successful write whose response was lost.
    pub async fn txn(&self, request: TxnRequest) -> Result<bool, Status> {
        let response = self
            .call(move |mut client| {
                let request = timed_request(request.clone());
                async move { client.txn(request).await }
            })
            .await?;
        Ok(response.succeeded)
    }

    async fn call<T, F>(&self, rpc: impl Fn(KvClient<Channel>) -> F) -> Result<T, Status>
    where
        F: std::future::Future<Output = Result<Response<T>, Status>>,
    {
        let preferred = self.0.preferred.load(Ordering::Relaxed);
        let rpc = &rpc;
        let attempt = |index: usize| async move {
            let response = tokio::time::timeout(DEADLINE, rpc(self.0.clients[index].clone())).await;
            match response {
                Ok(Ok(response)) => {
                    self.0.preferred.store(index, Ordering::Relaxed);
                    Some(Ok(response.into_inner()))
                }
                Ok(Err(status))
                    if !matches!(
                        status.code(),
                        Code::Unavailable | Code::DeadlineExceeded | Code::Cancelled
                    ) =>
                {
                    Some(Err(status))
                }
                _ => None,
            }
        };
        if let Some(result) = attempt(preferred).await {
            return result;
        }

        // Fan out only during a failure. This bounds recovery time even when
        // several client routes are partitioned from a healthy quorum.
        let mut fallback: FuturesUnordered<_> = (0..self.0.clients.len())
            .filter(|&index| index != preferred)
            .map(attempt)
            .collect();
        while let Some(result) = fallback.next().await {
            if let Some(result) = result {
                return result;
            }
        }
        Err(Status::unavailable("cluster directory unavailable"))
    }
}

fn timed_request<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request.set_timeout(DEADLINE);
    request
}
