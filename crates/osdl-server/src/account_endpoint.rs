use std::{future::Future, pin::Pin};

use osdl_proto::v1::{self as pb, osdl_server::Osdl};
use tokio::net::TcpListener;
use tokio_stream::{wrappers::TcpListenerStream, Stream};
use tonic::{Request, Response, Status};

use crate::{
    account::AccountAuth,
    auth::ListenerAuth,
    authorization::{
        info::AuthenticationService,
        stream::{expires, guard_stream},
        Authorizer, Permission,
    },
    ServeError,
};

type RpcStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

/// Serve an OpenSDL API with the same account scope and stream leases as a local engine.
pub async fn serve_account_endpoint(
    listener: TcpListener,
    account: AccountAuth,
    operations: impl Osdl,
    max_response_bytes: usize,
) -> Result<(), ServeError> {
    let instance = account.verifier.binding().instance.clone();
    let policy = ListenerAuth::Account(account.clone());
    policy
        .validate_listener(&instance, Some(listener.local_addr()?))
        .map_err(ServeError::Auth)?;
    let api = AccountApi {
        upstream: operations,
        authorizer: Authorizer::Account(account),
    };
    tonic::transport::Server::builder()
        .add_service(
            pb::authentication_server::AuthenticationServer::new(AuthenticationService {
                policy,
                instance,
            })
            .max_decoding_message_size(1024),
        )
        .add_service(
            pb::osdl_server::OsdlServer::new(api).max_encoding_message_size(max_response_bytes),
        )
        .serve_with_incoming(TcpListenerStream::new(listener))
        .await?;
    Ok(())
}

struct AccountApi<T> {
    upstream: T,
    authorizer: Authorizer,
}

impl<T> AccountApi<T> {
    fn request<B>(
        &self,
        request: Request<B>,
        permission: Permission,
    ) -> Result<Request<B>, Status> {
        self.authorizer.authorize(&request, permission)?;
        // The next hop is authorized by the control plane. It never receives the account token.
        Ok(Request::new(request.into_inner()))
    }

    async fn leased<B, V, S, F>(
        &self,
        request: Request<B>,
        open: impl FnOnce(Request<B>) -> F + Send,
    ) -> Result<Response<RpcStream<V>>, Status>
    where
        B: Send,
        F: Future<Output = Result<Response<S>, Status>> + Send,
        S: Stream<Item = Result<V, Status>> + Send + 'static,
        V: Send + 'static,
    {
        let lease = self.authorizer.authorize(&request, Permission::Read)?;
        tokio::select! {
            biased;
            status = expires(lease.as_ref()) => Err(status),
            result = open(Request::new(request.into_inner())) => {
                Ok(result?.map(|stream| guard_stream(Box::pin(stream), lease)))
            }
        }
    }
}

#[tonic::async_trait]
impl<T: Osdl> Osdl for AccountApi<T> {
    type StreamEventsStream = RpcStream<pb::Event>;
    type PublishMediaStream = RpcStream<pb::PublishMediaStatus>;

    async fn status(
        &self,
        r: Request<pb::StatusRequest>,
    ) -> Result<Response<pb::StatusResponse>, Status> {
        self.upstream
            .status(self.request(r, Permission::Read)?)
            .await
    }

    async fn list_nodes(
        &self,
        r: Request<pb::ListNodesRequest>,
    ) -> Result<Response<pb::ListNodesResponse>, Status> {
        self.upstream
            .list_nodes(self.request(r, Permission::Read)?)
            .await
    }

    async fn list_devices(
        &self,
        r: Request<pb::ListDevicesRequest>,
    ) -> Result<Response<pb::ListDevicesResponse>, Status> {
        self.upstream
            .list_devices(self.request(r, Permission::Read)?)
            .await
    }

    async fn get_device(
        &self,
        r: Request<pb::GetDeviceRequest>,
    ) -> Result<Response<pb::Device>, Status> {
        self.upstream
            .get_device(self.request(r, Permission::Read)?)
            .await
    }

    async fn wait_for_device(
        &self,
        r: Request<pb::WaitForDeviceRequest>,
    ) -> Result<Response<pb::Device>, Status> {
        let lease = self.authorizer.authorize(&r, Permission::Read)?;
        tokio::select! {
            biased;
            status = expires(lease.as_ref()) => Err(status),
            result = self.upstream.wait_for_device(Request::new(r.into_inner())) => result,
        }
    }

    async fn read_snapshot(
        &self,
        r: Request<pb::ReadSnapshotRequest>,
    ) -> Result<Response<pb::ReadSnapshotResponse>, Status> {
        self.upstream
            .read_snapshot(self.request(r, Permission::Read)?)
            .await
    }

    async fn send_command(
        &self,
        r: Request<pb::SendCommandRequest>,
    ) -> Result<Response<pb::CommandResult>, Status> {
        self.upstream
            .send_command(self.request(r, Permission::Operate)?)
            .await
    }

    async fn stream_events(
        &self,
        r: Request<pb::StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        self.leased(r, |req| self.upstream.stream_events(req)).await
    }

    async fn publish_media(
        &self,
        r: Request<pb::PublishMediaRequest>,
    ) -> Result<Response<Self::PublishMediaStream>, Status> {
        self.leased(r, |req| self.upstream.publish_media(req)).await
    }

    async fn shutdown(
        &self,
        r: Request<pb::ShutdownRequest>,
    ) -> Result<Response<pb::ShutdownResponse>, Status> {
        self.authorizer.authorize(&r, Permission::Shutdown)?;
        Err(Status::permission_denied("local_administrator_required"))
    }
}
