use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{
    daemon::{execute, Operation},
    AssetCache, PullResult,
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
    let cache = match cache_dir {
        Some(path) => AssetCache::open(path)?,
        None => AssetCache::discover()?,
    };
    let result: PullResult = execute(
        &cache,
        Operation::Pull {
            reference: args.reference,
            output: args.output.map(std::path::absolute).transpose()?,
            offline: args.offline,
            credentials: args.auth.credentials(),
        },
    )
    .await?;
    println!("{}", result.resolved_digest);
    Ok(())
}
