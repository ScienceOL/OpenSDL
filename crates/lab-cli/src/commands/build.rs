use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{
    daemon::DaemonClient, pack_directory, AssetCache, AssetReference, PackageDescriptor,
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
    let tag = args.tag.as_deref().map(AssetReference::parse).transpose()?;
    let result: PackageDescriptor = if let Some(client) = DaemonClient::discover(&cache).await? {
        client
            .call_json(
                "build",
                serde_json::json!({
                    "directory": args.directory,
                    "tag": tag.as_ref().map(ToString::to_string),
                    "output": args.output,
                }),
            )
            .await?
    } else {
        let temporary;
        let layout = if let Some(output) = args.output {
            pack_directory(&args.directory, &output)?;
            output
        } else {
            temporary = tempfile::tempdir_in(cache.root().join("staging"))?;
            let layout = temporary.path().join("layout");
            pack_directory(&args.directory, &layout)?;
            layout
        };
        cache.lock()?.import_layout(&layout, tag.as_ref())?
    };
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
