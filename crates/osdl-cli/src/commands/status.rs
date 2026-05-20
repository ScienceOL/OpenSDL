//! `osdl status` — quick health check of the running server.

use anyhow::Context;
use osdl_proto::v1 as pb;

use crate::client;

pub async fn run(opts: client::ClientOpts) -> anyhow::Result<()> {
    let paths = osdl_server::paths::Paths::discover()
        .map_err(|e| anyhow::anyhow!("paths: {e}"))?;
    let resolved = client::resolve(&opts, &paths)?;
    let mut client = client::connect(&resolved, &opts).await?;

    let resp = client
        .status(pb::StatusRequest {})
        .await
        .context("status RPC")?
        .into_inner();

    let engine = resp.engine.unwrap_or_default();
    let state_name = pb::engine_status::State::try_from(engine.state)
        .map(|s| s.as_str_name())
        .unwrap_or("UNKNOWN");

    println!("instance:    {}", resp.instance);
    println!("version:     {}", resp.version);
    println!("pid:         {}", resp.pid);
    if let Some(s) = resp.socket_path {
        println!("socket:      {}", s);
    }
    if let Some(a) = resp.listen_addr {
        println!("listen:      {}", a);
    }
    println!("engine:      {} (broker={})", state_name, engine.broker);
    println!("nodes:       {}", engine.node_count);
    println!("devices:     {}", engine.device_count);
    if !engine.error_message.is_empty() {
        println!("error:       {}", engine.error_message);
    }
    Ok(())
}
