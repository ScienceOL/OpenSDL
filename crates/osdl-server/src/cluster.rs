//! The cluster listener authenticates the control plane before serving engine RPCs.

use std::net::SocketAddr;
use std::sync::Arc;

use osdl_core::EngineHandle;
use osdl_proto::{cluster::v1 as api, v1::osdl_server::OsdlServer};
use tokio::{
    net::TcpListener,
    sync::{watch, Notify},
    task::JoinSet,
};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    transport::{Certificate, Identity, Server, ServerTlsConfig},
    Request, Response, Status,
};

use crate::{
    authorization::Authorizer, convert::now_ts, OsdlService, ServeError, ServerIdentity,
    SERVER_VERSION,
};

pub struct WorkerListener {
    name: String,
    listener: TcpListener,
    address: SocketAddr,
    server: Server,
}

impl WorkerListener {
    pub async fn bind(
        name: String,
        address: SocketAddr,
        identity: Identity,
        control_ca: Certificate,
    ) -> Result<Self, ServeError> {
        osdl_auth::crypto_provider();
        let server = Server::builder().tls_config(
            ServerTlsConfig::new()
                .identity(identity)
                .client_ca_root(control_ca),
        )?;
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        Ok(Self {
            name,
            listener,
            address,
            server,
        })
    }

    pub fn worker_name(&self) -> &str {
        &self.name
    }

    fn identity(&self) -> ServerIdentity {
        ServerIdentity {
            instance: self.name.clone(),
            version: SERVER_VERSION.into(),
            pid: std::process::id(),
            started_at: now_ts(),
            socket_path: None,
            listen_addr: Some(self.address.to_string()),
        }
    }

    /// Serve a worker without a direct client listener.
    pub async fn serve(self, engine: EngineHandle) -> Result<(), ServeError> {
        let (stop, stopped) = watch::channel(false);
        let shutdown = Arc::new(Notify::new());
        let service = OsdlService::with_shutdown(engine.clone(), self.identity(), shutdown.clone());
        let mut listeners = JoinSet::new();
        listeners.spawn(self.listen(engine.clone(), service, stopped));
        crate::supervise_listeners(engine, shutdown, stop, listeners).await
    }

    pub(crate) async fn listen(
        mut self,
        engine: EngineHandle,
        service: OsdlService,
        stopped: watch::Receiver<bool>,
    ) -> Result<(), tonic::transport::Error> {
        let service = service
            .with_identity(self.identity())
            .with_authorizer(Authorizer::ControlPlane);
        self.server
            .add_service(OsdlServer::new(service))
            .add_service(api::work_plane_server::WorkPlaneServer::new(Inventory(
                engine,
            )))
            .serve_with_incoming_shutdown(
                TcpListenerStream::new(self.listener),
                crate::listener_stopped(stopped),
            )
            .await
    }
}

struct Inventory(EngineHandle);

#[tonic::async_trait]
impl api::work_plane_server::WorkPlane for Inventory {
    async fn inventory(
        &self,
        _: Request<api::WorkerInventoryRequest>,
    ) -> Result<Response<api::DeviceInventory>, Status> {
        let devices = self
            .0
            .project_devices(|device| api::DeviceSummary {
                id: device.id.clone(),
                device_type: device.device_type.clone(),
                online: device.online,
                role: device.role.clone(),
            })
            .await;
        Ok(Response::new(api::DeviceInventory { devices }))
    }
}
