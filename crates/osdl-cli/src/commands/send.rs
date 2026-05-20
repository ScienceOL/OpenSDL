//! `osdl send <device> <action> [-p k=v ...]` — dispatch a single command.
//!
//! Note: the returned `status` is always `PENDING` today. The engine
//! hands the encoded bytes to the transport and returns immediately —
//! it does not correlate the device's reply back to the originating
//! command. For now, watch device state via `osdl device get` (or
//! `osdl events --kinds device_status`) to see the effect of a command;
//! treat `PENDING` as "command was dispatched without error", not "the
//! device confirmed completion". Tracked under issue: `SendCommand`
//! returns dispatch result, not completion.

use anyhow::{anyhow, Context};
use clap::Args;
use osdl_proto::v1 as pb;

use crate::client;

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Device id, e.g. `espnow:30EDA0B65B38:pump-1`.
    pub device_id: String,
    /// Action name (whatever the device's adapter exposes — see `osdl device get`).
    pub action: String,
    /// Parameter `k=v`. Repeat for multiple. Values are parsed as JSON when
    /// they look like JSON (numbers, true/false, "quoted strings",
    /// [arrays], {objects}); otherwise treated as a string.
    #[arg(short = 'p', long = "param", value_name = "K=V")]
    pub params: Vec<String>,
    /// Read params as a JSON object from this file. Mutually exclusive
    /// with `--param`. Use `-` for stdin.
    #[arg(long = "params-file", conflicts_with = "params")]
    pub params_file: Option<String>,
    /// Optional client-supplied command_id. If empty the server generates one.
    #[arg(long)]
    pub command_id: Option<String>,
    /// Print result as JSON instead of human-readable lines.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: SendArgs, opts: client::ClientOpts) -> anyhow::Result<()> {
    let params_value = if let Some(path) = &args.params_file {
        let raw = if path == "-" {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        } else {
            std::fs::read_to_string(path)
                .with_context(|| format!("read params file {path}"))?
        };
        serde_json::from_str::<serde_json::Value>(&raw).context("parse params file as JSON")?
    } else {
        let mut map = serde_json::Map::new();
        for kv in &args.params {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow!("--param must be K=V (got '{kv}')"))?;
            let val: serde_json::Value =
                serde_json::from_str(v).unwrap_or_else(|_| serde_json::Value::String(v.to_string()));
            map.insert(k.to_string(), val);
        }
        serde_json::Value::Object(map)
    };

    let pb_params = match &params_value {
        serde_json::Value::Object(_) => Some(osdl_server::convert::json_to_struct(&params_value)),
        serde_json::Value::Null => None,
        // Non-object scalars: wrap in an object under "_value" — the
        // server-side conversion expects `Struct`, not arbitrary `Value`.
        other => {
            let wrapped = serde_json::json!({ "_value": other });
            Some(osdl_server::convert::json_to_struct(&wrapped))
        }
    };

    let paths = osdl_server::paths::Paths::discover()
        .map_err(|e| anyhow::anyhow!("paths: {e}"))?;
    let resolved = client::resolve(&opts, &paths)?;
    let mut client = client::connect(&resolved, &opts).await?;

    let resp = client
        .send_command(pb::SendCommandRequest {
            command_id: args.command_id.unwrap_or_default(),
            device_id: args.device_id,
            action: args.action,
            params: pb_params,
        })
        .await
        .context("send_command RPC")?
        .into_inner();

    let status_name = pb::command_result::Status::try_from(resp.status)
        .map(|s| s.as_str_name())
        .unwrap_or("UNKNOWN");

    if args.json {
        let data = resp
            .data
            .as_ref()
            .map(osdl_server::convert::struct_to_json)
            .unwrap_or(serde_json::Value::Null);
        let json = serde_json::json!({
            "command_id": resp.command_id,
            "device_id": resp.device_id,
            "status": status_name,
            "message": resp.message,
            "data": data,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("command_id:  {}", resp.command_id);
        println!("device_id:   {}", resp.device_id);
        println!("status:      {}", status_name);
        println!("message:     {}", resp.message);
    }
    Ok(())
}
