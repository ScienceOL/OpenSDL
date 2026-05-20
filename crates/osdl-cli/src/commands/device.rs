//! `osdl device list|get|wait` — inspect known devices.

use std::time::Duration;

use anyhow::Context;
use clap::{Args, Subcommand};
use osdl_proto::v1 as pb;

use crate::client;

#[derive(Debug, Subcommand)]
pub enum DeviceCmd {
    /// List devices known to the engine.
    List(ListArgs),
    /// Look up a single device by id.
    Get(GetArgs),
    /// Block until a matching device appears.
    Wait(WaitArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Filter by adapter platform (e.g. `unilabos`).
    #[arg(long)]
    adapter: Option<String>,
    /// Filter by device_type.
    #[arg(long, value_name = "TYPE")]
    r#type: Option<String>,
    /// Filter by role (e.g. `stirrer`, `drain_valve`).
    #[arg(long)]
    role: Option<String>,
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct GetArgs {
    /// Device id, e.g. `espnow:30EDA0B65B38:pump-1`.
    pub device_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WaitArgs {
    /// What to wait for. Exactly one of: `id:DEVICE_ID`, `type:DEVICE_TYPE`, `role:ROLE`.
    pub selector: String,
    /// How long to wait, e.g. `15s`, `2m`. Default 30s.
    #[arg(long, value_parser = parse_duration, default_value = "30s")]
    pub timeout: Duration,
    #[arg(long)]
    pub json: bool,
}

pub async fn run(cmd: DeviceCmd, opts: client::ClientOpts) -> anyhow::Result<()> {
    let paths = osdl_server::paths::Paths::discover()
        .map_err(|e| anyhow::anyhow!("paths: {e}"))?;
    let resolved = client::resolve(&opts, &paths)?;
    let mut client = client::connect(&resolved, &opts).await?;

    match cmd {
        DeviceCmd::List(a) => {
            let resp = client
                .list_devices(pb::ListDevicesRequest {
                    adapter: a.adapter,
                    role: a.role,
                    device_type: a.r#type,
                })
                .await
                .context("list_devices RPC")?
                .into_inner();
            if a.json {
                let summaries: Vec<_> = resp.devices.iter().map(device_summary_json).collect();
                println!("{}", serde_json::to_string_pretty(&summaries)?);
            } else {
                print_device_table(&resp.devices);
            }
        }
        DeviceCmd::Get(a) => {
            let dev = client
                .get_device(pb::GetDeviceRequest {
                    device_id: a.device_id,
                })
                .await
                .context("get_device RPC")?
                .into_inner();
            if a.json {
                println!("{}", serde_json::to_string_pretty(&device_summary_json(&dev))?);
            } else {
                print_device_detail(&dev);
            }
        }
        DeviceCmd::Wait(a) => {
            let req = parse_selector(&a.selector, a.timeout)?;
            let dev = client
                .wait_for_device(req)
                .await
                .context("wait_for_device RPC")?
                .into_inner();
            if a.json {
                println!("{}", serde_json::to_string_pretty(&device_summary_json(&dev))?);
            } else {
                print_device_detail(&dev);
            }
        }
    }
    Ok(())
}

fn parse_selector(s: &str, timeout: Duration) -> anyhow::Result<pb::WaitForDeviceRequest> {
    let (key, val) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("selector must be `id:`, `type:`, or `role:` (got '{s}')"))?;
    let selector = match key {
        "id" => pb::wait_for_device_request::Selector::DeviceId(val.to_string()),
        "type" => pb::wait_for_device_request::Selector::DeviceType(val.to_string()),
        "role" => pb::wait_for_device_request::Selector::Role(val.to_string()),
        other => anyhow::bail!("unknown selector kind '{other}': use id, type, or role"),
    };
    Ok(pb::WaitForDeviceRequest {
        selector: Some(selector),
        timeout_ms: timeout.as_millis().min(u32::MAX as u128) as u32,
    })
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    if let Some(stripped) = s.strip_suffix("ms") {
        return stripped
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|e| e.to_string());
    }
    if let Some(stripped) = s.strip_suffix('s') {
        return stripped
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| e.to_string());
    }
    if let Some(stripped) = s.strip_suffix('m') {
        return stripped
            .parse::<u64>()
            .map(|m| Duration::from_secs(m * 60))
            .map_err(|e| e.to_string());
    }
    s.parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|e| e.to_string())
}

fn print_device_table(devices: &[pb::Device]) {
    if devices.is_empty() {
        println!("(no devices)");
        return;
    }
    println!(
        "{:<40}  {:<32}  {:<14}  {:<10}  {}",
        "DEVICE_ID", "DEVICE_TYPE", "ROLE", "ONLINE", "TRANSPORT"
    );
    for d in devices {
        println!(
            "{:<40}  {:<32}  {:<14}  {:<10}  {}",
            d.id,
            d.device_type,
            d.role.as_deref().unwrap_or("-"),
            if d.online { "yes" } else { "no" },
            d.transport_id,
        );
    }
}

fn print_device_detail(d: &pb::Device) {
    println!("id:           {}", d.id);
    println!("type:         {}", d.device_type);
    println!("adapter:      {}", d.adapter);
    println!("transport:    {}", d.transport_id);
    println!("role:         {}", d.role.as_deref().unwrap_or("-"));
    println!("online:       {}", d.online);
    println!("description:  {}", d.description);
    if !d.actions.is_empty() {
        println!("actions:");
        for a in &d.actions {
            println!("  - {} — {}", a.name, a.description);
        }
    }
    if let Some(props) = &d.properties {
        if !props.fields.is_empty() {
            let json = osdl_server::convert::struct_to_json(props);
            println!("properties:");
            for line in serde_json::to_string_pretty(&json).unwrap_or_default().lines() {
                println!("  {line}");
            }
        }
    }
}

fn device_summary_json(d: &pb::Device) -> serde_json::Value {
    let props = d
        .properties
        .as_ref()
        .map(osdl_server::convert::struct_to_json)
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "id": d.id,
        "device_type": d.device_type,
        "adapter": d.adapter,
        "transport_id": d.transport_id,
        "role": d.role,
        "online": d.online,
        "description": d.description,
        "actions": d.actions.iter().map(|a| serde_json::json!({
            "name": a.name,
            "description": a.description,
        })).collect::<Vec<_>>(),
        "properties": props,
    })
}
