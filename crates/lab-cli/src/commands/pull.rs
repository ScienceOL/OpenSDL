use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{
    daemon::DaemonClient, materialize, AssetCache, AssetReference, RegistryClient,
};

use super::registry_auth::RegistryAuthArgs;

#[derive(Args)]
pub struct PullArgs {
    /// Tagged or digest-pinned OCI asset reference.
    pub reference: String,
    /// Materialize the asset into this new directory after pulling.
    #[arg(long, short)]
    pub output: Option<PathBuf>,
    /// Resolve only from the local content-addressed cache.
    #[arg(long)]
    pub offline: bool,
    #[command(flatten)]
    pub auth: RegistryAuthArgs,
}

pub async fn run(args: PullArgs, cache_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let reference = AssetReference::parse(&args.reference)?;
    let cache = match cache_dir {
        Some(path) => AssetCache::open(path)?,
        None => AssetCache::discover()?,
    };
    if let Some(client) = DaemonClient::discover(&cache).await? {
        let result: serde_json::Value = client
            .call_json(
                "pull",
                serde_json::json!({
                    "reference": reference.to_string(),
                    "output": args.output,
                    "offline": args.offline,
                    "credentials": args.auth.credentials(),
                }),
            )
            .await?;
        if let Some(digest) = result
            .get("resolvedDigest")
            .and_then(serde_json::Value::as_str)
        {
            println!("{digest}");
        } else {
            println!("{result}");
        }
        return Ok(());
    }
    let result = RegistryClient::for_reference(&reference)
        .pull(&reference, &cache, &args.auth.credentials(), args.offline)
        .await?;
    if let Some(output) = args.output {
        materialize(&cache, &result.resolved_digest, &output)?;
    }
    println!("{}", result.resolved_digest);
    Ok(())
}
