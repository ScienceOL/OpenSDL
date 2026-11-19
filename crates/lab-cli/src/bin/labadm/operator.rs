use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;
use osdl_proto::{
    cluster::v1::{self as api, control_plane_client::ControlPlaneClient},
    v1 as osdl,
};
use tonic::{transport::Channel, Code, Status};

use crate::config::{self, Id, OperatorConfig};

type Client = ControlPlaneClient<Channel>;
const READY_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Subcommand)]
pub enum Command {
    /// Verify the cluster and show the selected control endpoint.
    Status,
    /// List admitted work-plane nodes.
    Workers,
    /// List devices across the cluster or on one worker.
    Devices {
        #[arg(long)]
        worker: Option<Id>,
    },
    /// Show a device and the actions its adapter exposes.
    Device { worker: Id, device_id: String },
    /// Dispatch one action to an explicitly selected worker.
    Send {
        worker: Id,
        device_id: String,
        action: String,
        /// A JSON object with the action's parameters.
        #[arg(long, default_value = "{}")]
        params: String,
        /// Correlate a dispatch attempt with the owning worker's local log.
        #[arg(long)]
        command_id: Option<String>,
    },
    /// Stream future events from the selected worker.
    Events {
        worker: Id,
        #[arg(long = "kind")]
        kinds: Vec<String>,
    },
    /// Read a previously captured camera snapshot from its owning worker.
    Snapshot {
        worker: Id,
        path: String,
        output: PathBuf,
    },
}

pub async fn run(command: Command, path: &Path) -> Result<()> {
    let config: OperatorConfig = config::read(path)?;
    let (mut client, endpoint) = connect(config, path).await?;
    match command {
        Command::Status => println!("ready at {endpoint}"),
        Command::Workers => list_workers(&mut client).await?,
        Command::Devices { worker } => list_devices(&mut client, worker).await?,
        Command::Device { worker, device_id } => {
            let device = bounded(client.get_device(api::DeviceRequest {
                worker: worker.to_string(),
                device_id,
            }))
            .await?
            .into_inner();
            let actions = device
                .actions
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "name": a.name,
                        "description": a.description,
                        "params": a.params.as_ref().map(osdl_server::convert::struct_to_json),
                    })
                })
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "worker": worker.as_str(),
                    "id": device.id,
                    "type": device.device_type,
                    "online": device.online,
                    "actions": actions,
                    "properties": device.properties.as_ref().map(osdl_server::convert::struct_to_json),
                }))?
            );
        }
        Command::Send {
            worker,
            device_id,
            action,
            params,
            command_id,
        } => {
            let params: serde_json::Value =
                serde_json::from_str(&params).context("invalid JSON parameters")?;
            ensure!(
                params.is_object(),
                "action parameters must be a JSON object"
            );
            let command_id = command_id
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let command = osdl::SendCommandRequest {
                command_id: command_id.clone(),
                device_id,
                action,
                params: Some(osdl_server::convert::json_to_struct(&params)),
            };
            let request = api::CommandRequest {
                worker: worker.to_string(),
                command: Some(command),
            };
            let result = bounded(client.send_command(request))
                .await
                .map_err(|error| match error.code() {
                    Code::Unknown
                    | Code::Unavailable
                    | Code::DeadlineExceeded
                    | Code::Cancelled => anyhow::anyhow!(
                        "command {command_id} outcome could not be confirmed: {error}"
                    ),
                    _ => error.into(),
                })?
                .into_inner();
            let status = osdl::command_result::Status::try_from(result.status)
                .unwrap_or(osdl::command_result::Status::Unspecified);
            println!(
                "{} {} {} {}",
                result.command_id,
                status.as_str_name(),
                result.device_id,
                result.message
            );
            ensure!(
                status != osdl::command_result::Status::Failed,
                "device dispatch failed"
            );
        }
        Command::Events { worker, kinds } => {
            let mut events = bounded(client.stream_events(api::EventsRequest {
                worker: worker.to_string(),
                filter: Some(osdl::StreamEventsRequest { kinds }),
            }))
            .await?
            .into_inner();
            while let Some(event) = events.message().await? {
                println!("{event:?}");
            }
        }
        Command::Snapshot {
            worker,
            path,
            output,
        } => {
            let image = bounded(client.read_snapshot(api::SnapshotRequest {
                worker: worker.to_string(),
                snapshot: Some(osdl::ReadSnapshotRequest { path }),
            }))
            .await?
            .into_inner();
            tokio::fs::write(output, image.jpeg).await?;
        }
    }
    Ok(())
}

async fn connect(config: OperatorConfig, path: &Path) -> Result<(Client, String)> {
    ensure!(
        !config.endpoints.is_empty(),
        "configure at least one control endpoint"
    );
    let tls = config.tls.load(path)?;
    let mut last_error = None;
    for address in config.endpoints {
        let endpoint = tls.endpoint(config::https_endpoint(address.clone())?, None)?;
        let mut client = ControlPlaneClient::new(endpoint.connect_lazy())
            .max_decoding_message_size(super::control::INVENTORY_RESPONSE_LIMIT);
        let ready = tokio::time::timeout(READY_TIMEOUT, client.ready(api::ReadyRequest {})).await;
        match ready {
            Ok(Ok(response)) if response.get_ref().cluster == config.cluster.as_str() => {
                return Ok((client, address))
            }
            Ok(Ok(_)) => last_error = Some(format!("{address} answered for a different cluster")),
            Ok(Err(error)) => last_error = Some(format!("{address}: {error}")),
            Err(_) => last_error = Some(format!("{address}: timed out")),
        }
    }
    bail!(
        "no ready control plane ({})",
        last_error.unwrap_or_default()
    )
}

async fn list_workers(client: &mut Client) -> Result<()> {
    let mut after = String::new();
    loop {
        let page = bounded(client.list_workers(api::WorkerPage { after }))
            .await?
            .into_inner();
        for worker in page.workers {
            println!("{worker}");
        }
        after = page.next_after;
        if after.is_empty() {
            return Ok(());
        }
    }
}

async fn list_devices(client: &mut Client, worker: Option<Id>) -> Result<()> {
    let worker = worker.map(|id| id.to_string()).unwrap_or_default();
    let mut after = String::new();
    loop {
        let page = bounded(client.list_devices(api::InventoryRequest {
            worker: worker.clone(),
            after,
        }))
        .await?
        .into_inner();
        for item in page.workers {
            if let Some(error) = item.unavailable {
                println!("{}\tunavailable\t{}", item.worker, error);
            } else if item.devices.is_empty() {
                println!("{}\t(no devices)", item.worker);
            } else {
                for device in item.devices {
                    println!(
                        "{}\t{}\t{}\t{}",
                        item.worker,
                        device.id,
                        device.device_type,
                        if device.online { "online" } else { "offline" }
                    );
                }
            }
        }
        after = page.next_after;
        if after.is_empty() {
            return Ok(());
        }
    }
}

async fn bounded<T>(
    rpc: impl std::future::Future<Output = Result<T, Status>>,
) -> Result<T, Status> {
    tokio::time::timeout(REQUEST_TIMEOUT, rpc)
        .await
        .map_err(|_| Status::deadline_exceeded("control request timed out"))?
}
