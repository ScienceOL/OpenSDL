use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{inspect_layout, VirtualAssetManifest};

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Portable asset directory or OCI image-layout directory.
    pub path: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: InspectArgs) -> anyhow::Result<()> {
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

    let package = inspect_layout(&args.path)?;
    if args.json {
        println!("{}", serde_json::to_string(&package)?);
    } else {
        println!("{}", package.manifest_digest);
    }
    Ok(())
}
