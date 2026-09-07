use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{materialize, AssetCache, AssetReference, RegistryClient};

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
    /// Override the platform cache directory.
    #[arg(long, env = "LAB_ASSET_CACHE")]
    pub cache_dir: Option<PathBuf>,
    #[command(flatten)]
    pub auth: RegistryAuthArgs,
}

pub async fn run(args: PullArgs) -> anyhow::Result<()> {
    let reference = AssetReference::parse(&args.reference)?;
    let cache = match args.cache_dir {
        Some(path) => AssetCache::open(path)?,
        None => AssetCache::discover()?,
    };
    let result = RegistryClient::for_reference(&reference)
        .pull(&reference, &cache, &args.auth.credentials(), args.offline)
        .await?;
    if let Some(output) = args.output {
        materialize(&cache, &result.resolved_digest, &output)?;
    }
    println!("{}", result.resolved_digest);
    Ok(())
}
