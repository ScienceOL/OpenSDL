use std::path::PathBuf;

use clap::Args;
use opensdl_assets::VirtualAssetManifest;

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Directory containing asset.yaml and all declared files.
    pub directory: PathBuf,
}

pub fn run(args: ValidateArgs) -> anyhow::Result<()> {
    let asset = VirtualAssetManifest::load(&args.directory)?;
    let capabilities = asset
        .manifest
        .spec
        .capabilities
        .iter()
        .map(|capability| format!("{capability:?}").to_lowercase())
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "{} {} ({:?}; {})",
        asset.manifest.metadata.name,
        asset.manifest.metadata.version,
        asset.manifest.spec.source.format,
        capabilities
    );
    Ok(())
}
