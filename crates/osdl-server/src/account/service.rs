use super::{lease::Leases, Binding, Verifier};
use osdl_proto::v1 as pb;
use tonic::{Request, Response, Status};
#[derive(Clone, Debug)]
pub struct AccountAuth {
    pub(crate) verifier: Verifier,
    pub(crate) leases: Leases,
}
impl AccountAuth {
    pub fn new(binding: Binding) -> anyhow::Result<Self> {
        Ok(Self {
            verifier: Verifier::new(binding)?,
            leases: Leases::default(),
        })
    }
}
#[derive(Clone)]
pub(crate) struct AuthenticationService {
    pub account: Option<AccountAuth>,
    pub mode: String,
    pub instance: String,
}
#[tonic::async_trait]
impl pb::authentication_server::Authentication for AuthenticationService {
    async fn info(
        &self,
        _: Request<pb::AuthenticationInfoRequest>,
    ) -> Result<Response<pb::AuthenticationInfoResponse>, Status> {
        let binding = self
            .account
            .as_ref()
            .map(|account| account.verifier.binding());
        Ok(Response::new(pb::AuthenticationInfoResponse {
            mode: self.mode.clone(),
            instance: self.instance.clone(),
            lab_id: binding.map(|b| b.lab_id.clone()).unwrap_or_default(),
            issuer: binding.map(|b| b.issuer.clone()).unwrap_or_default(),
            channel: binding.map(|b| b.channel.clone()).unwrap_or_default(),
        }))
    }
    async fn renew(
        &self,
        request: Request<pb::AuthenticationRenewRequest>,
    ) -> Result<Response<pb::AuthenticationRenewResponse>, Status> {
        let account = self.account.as_ref().ok_or_else(|| {
            Status::failed_precondition("account authentication is not configured")
        })?;
        let identity = account.verifier.authenticate(&request)?;
        account.leases.renew(&identity);
        Ok(Response::new(pb::AuthenticationRenewResponse {
            expires_at: identity.expires_at as i64,
        }))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Permission {
    Read,
    Operate,
    Shutdown,
}
pub(crate) fn authorize<T>(
    account: Option<&AccountAuth>,
    request: &Request<T>,
    permission: Permission,
) -> Result<Option<super::Identity>, Status> {
    let Some(account) = account else {
        return Ok(None);
    };
    let identity = account.verifier.authenticate(request)?;
    match permission {
        Permission::Shutdown => {
            return Err(Status::permission_denied(
                "account authorization cannot shut down the laboratory",
            ))
        }
        Permission::Operate if identity.scope != super::Scope::Operate => {
            return Err(Status::permission_denied(
                "laboratory operation permission required",
            ))
        }
        _ => {}
    }
    Ok(Some(identity))
}
pub(crate) async fn expires(lease: Option<&std::sync::Arc<super::lease::Lease>>) {
    match lease {
        Some(lease) => lease.expired().await,
        None => std::future::pending::<()>().await,
    }
}
pub(crate) fn guard_stream<T: Send + 'static>(
    mut stream: std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>,
    lease: Option<std::sync::Arc<super::lease::Lease>>,
) -> std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>> {
    use tokio_stream::StreamExt;
    let Some(lease) = lease else { return stream };
    Box::pin(async_stream::stream! {
        let expired = lease.expired();
        tokio::pin!(expired);
        loop {
            tokio::select! {
                biased;
                _ = &mut expired => {
                    yield Err(super::identity::AuthError::Expired.status());
                    break;
                },
                item = stream.next() => match item {
                    Some(item) => yield item,
                    None => break,
                },
            }
        }
    })
}
