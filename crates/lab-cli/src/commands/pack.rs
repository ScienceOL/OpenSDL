use std::path::PathBuf;

use clap::Args;

#[derive(Debug, Args)]
pub struct PackArgs {
    /// Directory containing the portable asset.
    pub directory: PathBuf,
    /// New OCI image-layout directory to create.
    #[arg(long, short)]
    pub output: PathBuf,
}

pub fn run(args: PackArgs) -> anyhow::Result<()> {
    let package =
        osdl_server::simulation_validation::pack_directory(&args.directory, &args.output)?;
    println!("{}", package.manifest_digest);
    Ok(())
}
