use std::path::{Path, PathBuf};

use clap::Args;
use opensdl_assets::{
    daemon::{execute, Operation},
    AssetCache, AssetReference, PushResult, RegistryClient,
};

use super::registry_auth::RegistryAuthArgs;

#[derive(Args)]
pub struct PushArgs {
    /// OCI image-layout directory, or a local tag/digest admitted to the asset store.
    pub source: String,
    /// Tagged OCI destination, for example localhost:5001/lab/ur5e:1.0.0.
    pub reference: String,
    #[command(flatten)]
    pub auth: RegistryAuthArgs,
}

pub async fn run(args: PushArgs, cache_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let reference = AssetReference::parse(&args.reference)?;
    let result = if Path::new(&args.source).is_dir() {
        RegistryClient::for_reference(&reference)
            .push_layout(
                Path::new(&args.source),
                &reference,
                &args.auth.credentials(),
            )
            .await?
    } else {
        let cache = match cache_dir {
            Some(path) => AssetCache::open(path)?,
            None => AssetCache::discover()?,
        };
        execute::<PushResult>(
            &cache,
            Operation::Push {
                source: args.source,
                reference: reference.to_string(),
                credentials: args.auth.credentials(),
            },
        )
        .await?
    };
    println!("{}", result.manifest_digest);
    Ok(())
}
