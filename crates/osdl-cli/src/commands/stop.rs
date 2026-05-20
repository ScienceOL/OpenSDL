//! `osdl stop` — ask the running server to shut down.

use anyhow::Context;
use osdl_proto::v1 as pb;

use crate::client;

pub async fn run(opts: client::ClientOpts) -> anyhow::Result<()> {
    let paths = osdl_server::paths::Paths::discover()
        .map_err(|e| anyhow::anyhow!("paths: {e}"))?;
    let resolved = client::resolve(&opts, &paths)?;
    let mut client = client::connect(&resolved, &opts).await?;
    client
        .shutdown(pb::ShutdownRequest::default())
        .await
        .context("shutdown RPC")?;
    println!("shutdown requested");
    Ok(())
}
