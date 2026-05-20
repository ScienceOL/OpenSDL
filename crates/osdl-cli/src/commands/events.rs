//! `osdl events [--follow]` — stream engine events.

use anyhow::Context;
use clap::Args;
use osdl_proto::v1 as pb;
use tokio_stream::StreamExt;

use crate::client;

#[derive(Debug, Args)]
pub struct EventsArgs {
    /// Stream events as they arrive instead of returning after the first one.
    /// (Currently the only mode; reserved for future snapshot-style listing.)
    #[arg(long, default_value_t = true)]
    pub follow: bool,
    /// Filter to specific event kinds. Comma-separated. Valid kinds:
    /// `device_online`, `device_offline`, `device_status`, `command_result`,
    /// `unknown_node`, `media_source_online`, `media_gateway_down`, `lagged`.
    #[arg(long, value_delimiter = ',')]
    pub kinds: Vec<String>,
    /// Output each event as one JSON line (jsonl) instead of human-readable.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: EventsArgs, opts: client::ClientOpts) -> anyhow::Result<()> {
    let _ = args.follow; // currently always-follow; silenced lint

    let paths = osdl_server::paths::Paths::discover()
        .map_err(|e| anyhow::anyhow!("paths: {e}"))?;
    let resolved = client::resolve(&opts, &paths)?;
    let mut client = client::connect(&resolved, &opts).await?;

    let resp = client
        .stream_events(pb::StreamEventsRequest { kinds: args.kinds })
        .await
        .context("stream_events RPC")?;
    let mut stream = resp.into_inner();

    while let Some(item) = stream.next().await {
        let ev = item.context("event recv")?;
        if args.json {
            println!("{}", event_to_jsonl(&ev));
        } else {
            print_human(&ev);
        }
    }
    Ok(())
}

fn print_human(ev: &pb::Event) {
    let ts = ev
        .timestamp
        .as_ref()
        .map(|t| format!("{}.{:03}", t.seconds, (t.nanos / 1_000_000).max(0)))
        .unwrap_or_default();
    match &ev.kind {
        Some(pb::event::Kind::DeviceOnline(e)) => {
            if let Some(d) = &e.device {
                println!(
                    "[{ts}] device_online    {}  type={}  role={}",
                    d.id,
                    d.device_type,
                    d.role.as_deref().unwrap_or("-")
                );
            }
        }
        Some(pb::event::Kind::DeviceOffline(e)) => {
            println!("[{ts}] device_offline   {}", e.device_id);
        }
        Some(pb::event::Kind::DeviceStatus(e)) => {
            let props = e
                .properties
                .as_ref()
                .map(osdl_server::convert::struct_to_json)
                .unwrap_or(serde_json::Value::Null);
            println!(
                "[{ts}] device_status    {}  {}",
                e.device_id,
                serde_json::to_string(&props).unwrap_or_default()
            );
        }
        Some(pb::event::Kind::CommandResult(e)) => {
            if let Some(r) = &e.result {
                let status = pb::command_result::Status::try_from(r.status)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN");
                println!(
                    "[{ts}] command_result   {}  device={}  status={}  msg={}",
                    r.command_id, r.device_id, status, r.message
                );
            }
        }
        Some(pb::event::Kind::UnknownNode(e)) => {
            println!(
                "[{ts}] unknown_node     {}  hardware_id={}",
                e.node_id, e.hardware_id
            );
        }
        Some(pb::event::Kind::MediaSourceOnline(e)) => {
            println!(
                "[{ts}] media_source     {} ({})  endpoints={}",
                e.id,
                e.description,
                e.endpoints.len()
            );
            for ep in &e.endpoints {
                println!("                   [{}] {}  {}", ep.location, ep.protocol, ep.url);
            }
        }
        Some(pb::event::Kind::MediaGatewayDown(e)) => {
            println!("[{ts}] media_gateway_down  {}", e.reason);
        }
        Some(pb::event::Kind::Lagged(e)) => {
            println!("[{ts}] lagged           dropped={}", e.dropped);
        }
        None => {
            eprintln!("[{ts}] (event with no kind — server bug?)");
        }
    }
}

fn event_to_jsonl(ev: &pb::Event) -> String {
    let mut obj = serde_json::Map::new();
    if let Some(t) = &ev.timestamp {
        obj.insert(
            "timestamp_ms".into(),
            serde_json::Value::from(t.seconds * 1000 + (t.nanos as i64) / 1_000_000),
        );
    }
    let (kind, payload) = match &ev.kind {
        Some(pb::event::Kind::DeviceOnline(e)) => (
            "device_online",
            e.device.as_ref().map(device_to_json).unwrap_or(serde_json::Value::Null),
        ),
        Some(pb::event::Kind::DeviceOffline(e)) => (
            "device_offline",
            serde_json::json!({"device_id": e.device_id}),
        ),
        Some(pb::event::Kind::DeviceStatus(e)) => (
            "device_status",
            serde_json::json!({
                "device_id": e.device_id,
                "properties": e.properties.as_ref().map(osdl_server::convert::struct_to_json),
            }),
        ),
        Some(pb::event::Kind::CommandResult(e)) => (
            "command_result",
            e.result
                .as_ref()
                .map(|r| {
                    serde_json::json!({
                        "command_id": r.command_id,
                        "device_id": r.device_id,
                        "status": pb::command_result::Status::try_from(r.status)
                            .map(|s| s.as_str_name())
                            .unwrap_or("UNKNOWN"),
                        "message": r.message,
                    })
                })
                .unwrap_or(serde_json::Value::Null),
        ),
        Some(pb::event::Kind::UnknownNode(e)) => (
            "unknown_node",
            serde_json::json!({"node_id": e.node_id, "hardware_id": e.hardware_id}),
        ),
        Some(pb::event::Kind::MediaSourceOnline(e)) => (
            "media_source_online",
            serde_json::json!({
                "id": e.id,
                "description": e.description,
                "endpoints": e.endpoints.iter().map(|ep| serde_json::json!({
                    "protocol": ep.protocol,
                    "location": ep.location,
                    "url": ep.url,
                })).collect::<Vec<_>>(),
            }),
        ),
        Some(pb::event::Kind::MediaGatewayDown(e)) => (
            "media_gateway_down",
            serde_json::json!({"reason": e.reason}),
        ),
        Some(pb::event::Kind::Lagged(e)) => (
            "lagged",
            serde_json::json!({"dropped": e.dropped}),
        ),
        None => ("unknown", serde_json::Value::Null),
    };
    obj.insert("kind".into(), serde_json::Value::String(kind.into()));
    obj.insert("payload".into(), payload);
    serde_json::Value::Object(obj).to_string()
}

fn device_to_json(d: &pb::Device) -> serde_json::Value {
    serde_json::json!({
        "id": d.id,
        "device_type": d.device_type,
        "adapter": d.adapter,
        "transport_id": d.transport_id,
        "role": d.role,
        "online": d.online,
        "description": d.description,
    })
}
