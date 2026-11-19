use std::sync::Arc;

use lab_cli::runtime::{EngineArgs, EngineMode};
use osdl_core::EngineHandle;
use osdl_proto::{cluster::v1 as api, v1::osdl_server::OsdlServer};
use osdl_server::{
    authorization::Authorizer, convert::now_ts, OsdlService, ServerIdentity, SERVER_VERSION,
};
use tokio::{net::TcpListener, sync::Notify};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{transport::Server, Request, Response, Status};

use crate::{config::WorkerConfig, tls::Tls};

pub async fn serve(config: WorkerConfig, tls: Tls, engine_start: EngineArgs) -> anyhow::Result<()> {
    let server = Server::builder().tls_config(tls.server())?;
    let listener = TcpListener::bind(config.listen).await?;
    let identity = ServerIdentity {
        instance: config.worker.to_string(),
        version: SERVER_VERSION.into(),
        pid: std::process::id(),
        started_at: now_ts(),
        socket_path: None,
        listen_addr: Some(listener.local_addr()?.to_string()),
    };
    let paths = osdl_server::Paths::discover().map_err(anyhow::Error::msg)?;
    let instance = config.worker.to_string();
    lab_cli::runtime::run_engine(
        engine_start,
        &instance,
        paths.state_dir,
        EngineMode::WorkPlane,
        |engine| serve_engine(server, listener, engine, identity),
    )
    .await
}

async fn serve_engine(
    mut server: Server,
    listener: TcpListener,
    engine: EngineHandle,
    identity: ServerIdentity,
) -> anyhow::Result<()> {
    let shutdown = Arc::new(Notify::new());
    let service = OsdlService::with_shutdown(engine.clone(), identity, shutdown.clone())
        .with_authorizer(Authorizer::ControlPlane);
    server
        .add_service(OsdlServer::new(service))
        .add_service(api::work_plane_server::WorkPlaneServer::new(Inventory(
            engine.clone(),
        )))
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
            tokio::select! {
                () = engine.stop_requested() => {},
                result = tokio::signal::ctrl_c() => {
                    if let Err(error) = result {
                        log::warn!("console signal unavailable: {error}");
                        engine.stop_requested().await;
                    }
                }
            }
            shutdown.notify_waiters();
            shutdown.notify_one();
        })
        .await?;
    Ok(())
}

#[derive(Clone)]
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
