use osdl_proto::v1 as pb;
use tonic::{Request, Response, Status};
#[derive(Clone)]
pub(crate) struct AuthenticationService {
    pub policy: crate::auth::ListenerAuth,
    pub instance: String,
}
#[tonic::async_trait]
impl pb::authentication_server::Authentication for AuthenticationService {
    async fn info(
        &self,
        _: Request<pb::AuthenticationInfoRequest>,
    ) -> Result<Response<pb::AuthenticationInfoResponse>, Status> {
        let (lab_id, issuer, channel) = match &self.policy {
            crate::auth::ListenerAuth::Account(account) => {
                let b = account.verifier.binding();
                (b.lab_id.clone(), b.issuer.clone(), b.channel.clone())
            }
            crate::auth::ListenerAuth::PublicKey(key) => {
                (key.lab_id().into(), String::new(), key.channel().into())
            }
            _ => (String::new(), String::new(), String::new()),
        };
        Ok(Response::new(pb::AuthenticationInfoResponse {
            mode: self.policy.mode().into(),
            instance: self.instance.clone(),
            lab_id,
            issuer,
            channel,
        }))
    }
    async fn renew(
        &self,
        request: Request<pb::AuthenticationRenewRequest>,
    ) -> Result<Response<pb::AuthenticationRenewResponse>, Status> {
        let crate::auth::ListenerAuth::Account(account) = &self.policy else {
            return Err(Status::failed_precondition(
                "account authentication is not configured",
            ));
        };
        let identity = account.verifier.authenticate(&request)?;
        account.leases.renew(&identity);
        Ok(Response::new(pb::AuthenticationRenewResponse {
            expires_at: identity.expires_at as i64,
        }))
    }
}
