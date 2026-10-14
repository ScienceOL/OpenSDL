use std::path::{Path, PathBuf};

use clap::Args;
use opensdl_assets::{daemon::DaemonClient, AssetCache, AssetReference, RegistryClient};

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
    let cache = match cache_dir {
        Some(path) => AssetCache::open(path)?,
        None => AssetCache::discover()?,
    };
    if let Some(client) = DaemonClient::discover(&cache).await? {
        let result: opensdl_assets::PushResult = client
            .call_json(
                "push",
                serde_json::json!({
                    "source": args.source,
                    "reference": reference.to_string(),
                    "credentials": args.auth.credentials(),
                }),
            )
            .await?;
        println!("{}", result.manifest_digest);
        return Ok(());
    }
    let result = if PathBuf::from(&args.source).is_dir() {
        RegistryClient::for_reference(&reference)
            .push_layout(
                Path::new(&args.source),
                &reference,
                &args.auth.credentials(),
            )
            .await?
    } else {
        RegistryClient::for_reference(&reference)
            .push_cached(&cache, &args.source, &reference, &args.auth.credentials())
            .await?
    };
    println!("{}", result.manifest_digest);
    Ok(())
}
