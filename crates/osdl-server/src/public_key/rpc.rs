use super::PublicKeyAuth;
use osdl_proto::v1 as pb;
use tonic::{Request, Response, Status};
fn error(e: anyhow::Error) -> Status {
    // Closed error vocabulary: never format credentials or SQLite/native errors remotely.
    match e.to_string().as_str() {
        "invitation_consumed" => Status::already_exists("invitation_consumed"),
        "invitation_expired" => Status::unauthenticated("invitation_expired"),
        "grant_revoked" => Status::unauthenticated("grant_revoked"),
        "grant_expired" => Status::unauthenticated("grant_expired"),
        "invalid_csr" | "unsupported_csr_algorithm" => Status::invalid_argument("invalid_csr"),
        "invitation_limit" => Status::resource_exhausted("invitation_limit"),
        _ => Status::invalid_argument("invalid_enrollment_request"),
    }
}
async fn store_work<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Status::internal("authorization_store_failed"))?
        .map_err(error)
}
async fn enrollment<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, Status> {
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
    let permit = SLOTS
        .try_acquire()
        .map_err(|_| Status::resource_exhausted("pairing_busy"))?;
    store_work(move || {
        // Cancelled callers still own capacity until their blocking work exits.
        let _permit = permit;
        work()
    })
    .await
}
#[tonic::async_trait]
impl pb::pairing_server::Pairing for PublicKeyAuth {
    async fn describe_invitation(
        &self,
        request: Request<pb::DescribeInvitationRequest>,
    ) -> Result<Response<pb::DescribeInvitationResponse>, Status> {
        let r = request.into_inner();
        let auth = self.clone();
        enrollment(move || auth.describe(&r.invitation_id, &r.capability))
            .await
            .map(Response::new)
    }
    async fn pair(
        &self,
        request: Request<pb::PairRequest>,
    ) -> Result<Response<pb::PairResponse>, Status> {
        let auth = self.clone();
        let request = request.into_inner();
        enrollment(move || auth.pair(&request))
            .await
            .map(Response::new)
    }
}
#[tonic::async_trait]
impl pb::grant_administration_server::GrantAdministration for PublicKeyAuth {
    async fn invite(
        &self,
        request: Request<pb::InviteRequest>,
    ) -> Result<Response<pb::InviteResponse>, Status> {
        let r = request.into_inner();
        let access = match r.access {
            1 => osdl_auth::Access::Read,
            2 => osdl_auth::Access::Operate,
            _ => return Err(Status::invalid_argument("invalid_access")),
        };
        let expiry = r
            .expires_at
            .map(u64::try_from)
            .transpose()
            .map_err(|_| Status::invalid_argument("invalid_expiry"))?;
        let auth = self.clone();
        let invitation =
            store_work(move || auth.invite(access, expiry).and_then(|i| i.encode())).await?;
        Ok(Response::new(pb::InviteResponse { invitation }))
    }
    async fn list_grants(
        &self,
        request: Request<pb::ListGrantsRequest>,
    ) -> Result<Response<pb::ListGrantsResponse>, Status> {
        let r = request.into_inner();
        let auth = self.clone();
        store_work(move || auth.list(&r.after_id, r.limit))
            .await
            .map(Response::new)
    }
    async fn revoke(
        &self,
        request: Request<pb::RevokeGrantRequest>,
    ) -> Result<Response<pb::RevokeGrantResponse>, Status> {
        self.revoke(&request.into_inner().id).await.map_err(error)?;
        Ok(Response::new(pb::RevokeGrantResponse {}))
    }
}
