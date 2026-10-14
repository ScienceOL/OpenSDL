use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{
    daemon::DaemonClient, inspect_layout, AssetCache, PackageDescriptor, VirtualAssetManifest,
};

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Portable asset directory, OCI image-layout directory, or local tag/digest.
    pub path: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: InspectArgs, cache_dir: Option<PathBuf>) -> anyhow::Result<()> {
    if args.path.join("asset.yaml").is_file() {
        let asset = VirtualAssetManifest::load(&args.path)?;
        if args.json {
            println!("{}", serde_json::to_string(&asset.manifest)?);
        } else {
            println!(
                "{} {} ({:?})",
                asset.manifest.metadata.name,
                asset.manifest.metadata.version,
                asset.manifest.spec.source.format
            );
        }
        return Ok(());
    }

    if args.path.join("oci-layout").is_file() {
        let package = inspect_layout(&args.path)?;
        if args.json {
            println!("{}", serde_json::to_string(&package)?);
        } else {
            println!("{}", package.manifest_digest);
        }
        return Ok(());
    }

    let cache = match cache_dir {
        Some(path) => AssetCache::open(path)?,
        None => AssetCache::discover()?,
    };
    let package: PackageDescriptor = if let Some(client) = DaemonClient::discover(&cache).await? {
        client
            .call_json("inspect", serde_json::json!({"reference": args.path}))
            .await?
    } else {
        cache.inspect(&args.path.to_string_lossy())?
    };
    if args.json {
        println!("{}", serde_json::to_string(&package)?);
    } else {
        println!(
            "{} {} {}",
            package.manifest_digest, package.asset.metadata.name, package.asset.metadata.version
        );
    }
    Ok(())
}
