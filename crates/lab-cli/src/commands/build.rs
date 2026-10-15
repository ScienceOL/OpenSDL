use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{
    daemon::{execute, Operation},
    AssetCache, PackageDescriptor,
};

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Directory containing the portable OpenSDL asset.
    pub directory: PathBuf,
    /// Local tag to assign after the package is admitted.
    #[arg(short = 't', long)]
    pub tag: Option<String>,
    /// Also retain the generated OCI image layout at this path.
    #[arg(long, short)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: BuildArgs, root: Option<PathBuf>) -> anyhow::Result<()> {
    let cache = match root {
        Some(root) => AssetCache::open(root)?,
        None => AssetCache::discover()?,
    };
    let tag = args.tag;
    let result: PackageDescriptor = execute(
        &cache,
        Operation::Build {
            directory: std::path::absolute(args.directory)?,
            output: args.output.map(std::path::absolute).transpose()?,
            tag: tag.clone(),
        },
    )
    .await?;
    if args.json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        println!("{}", result.manifest_digest);
        if let Some(tag) = tag {
            println!("tagged {}", tag);
        }
    }
    Ok(())
}
