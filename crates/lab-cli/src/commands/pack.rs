use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use opensdl_assets::pack_directory;

#[derive(Debug, Args)]
pub struct PackArgs {
    /// Directory containing the portable asset.
    pub directory: PathBuf,
    /// New OCI image-layout directory to create.
    #[arg(long, short)]
    pub output: PathBuf,
}

pub fn run(args: PackArgs) -> anyhow::Result<()> {
    osdl_server::simulation_validation::validate_directory(&args.directory)
        .context("cannot pack an asset that failed validation")?;
    let package = pack_directory(&args.directory, &args.output)?;
    println!("{}", package.manifest_digest);
    Ok(())
}
