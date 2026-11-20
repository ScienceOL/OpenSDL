use clap::Subcommand;
use opensdl_assets::{
    daemon::{execute, Operation},
    AssetCache, AssetReference, AssetSummary, PackageDescriptor, PruneReport, RegistryClient,
    RegistryCredentials,
};
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum AssetCmd {
    /// Populate the local asset library before launching a scene.
    PrepareScene { scene: PathBuf },
    /// List locally admitted packages and their references.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Import a validated OCI layout into the local store.
    Import {
        layout: PathBuf,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Assign a local tag to a cached package.
    Tag { source: String, target: String },
    /// Inspect and verify a locally cached package.
    Inspect {
        reference: String,
        #[arg(long)]
        json: bool,
    },
    /// Remove a local reference or an unoccupied package digest.
    Remove { reference: String },
    /// Reclaim unreferenced and unoccupied packages and blobs.
    Prune {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(command: AssetCmd, root: Option<PathBuf>) -> anyhow::Result<()> {
    let cache = match root {
        Some(root) => AssetCache::open(root)?,
        None => AssetCache::discover()?,
    };
    match command {
        AssetCmd::PrepareScene { scene } => {
            let scene = osdl_server::simulation_asset::SceneDefinition::read(&scene)?;
            let refs = scene
                .instruments
                .iter()
                .map(|item| AssetReference::parse(&item.asset))
                .collect::<Result<Vec<_>, _>>()?;
            let digests = acquire(&cache, refs).await?;
            for digest in digests {
                cache.pin(&format!("scene:{digest}"), &digest)?;
            }
        }
        AssetCmd::List { json } => {
            let assets: Vec<AssetSummary> = execute(&cache, Operation::List).await?;
            if json {
                println!("{}", serde_json::to_string(&assets)?);
            } else {
                for asset in assets {
                    println!(
                        "{} {} {} {} bytes {}",
                        asset.manifest_digest,
                        asset.name,
                        asset.version,
                        asset.size_bytes,
                        asset.references.join(", ")
                    );
                }
            }
        }
        AssetCmd::Import { layout, tag } => {
            let package: PackageDescriptor = execute(
                &cache,
                Operation::Import {
                    layout: std::path::absolute(layout)?,
                    tag,
                },
            )
            .await?;
            println!("{}", package.manifest_digest);
        }
        AssetCmd::Tag { source, target } => {
            execute::<()>(&cache, Operation::Tag { source, target }).await?
        }
        AssetCmd::Inspect { reference, json } => {
            let package: PackageDescriptor =
                execute(&cache, Operation::Inspect { reference }).await?;
            if json {
                println!("{}", serde_json::to_string(&package)?);
            } else {
                println!(
                    "{} {} {}",
                    package.manifest_digest,
                    package.asset.metadata.name,
                    package.asset.metadata.version
                );
            }
        }
        AssetCmd::Remove { reference } => {
            execute::<()>(&cache, Operation::Remove { reference }).await?
        }
        AssetCmd::Prune { dry_run, json } => {
            let report: PruneReport = execute(&cache, Operation::Prune { dry_run }).await?;
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                println!(
                    "{} packages, {} blobs, {} bytes{}",
                    report.packages.len(),
                    report.blobs.len(),
                    report.reclaimed_bytes,
                    if dry_run { " (dry run)" } else { " reclaimed" }
                );
            }
        }
    }
    Ok(())
}

pub(super) async fn acquire(
    cache: &AssetCache,
    references: Vec<AssetReference>,
) -> anyhow::Result<std::collections::HashSet<String>> {
    use anyhow::Context;
    let mut pulls = tokio::task::JoinSet::new();
    let mut requested = std::collections::HashSet::new();
    for reference in references {
        let digest = reference.digest().context("scene requires pinned assets")?;
        if !requested.insert(digest.to_owned()) || cache.package(digest).is_ok() {
            continue;
        }
        let cache = cache.clone();
        pulls.spawn(async move {
            RegistryClient::for_reference(&reference)
                .pull(&reference, &cache, &RegistryCredentials::Anonymous, false)
                .await
        });
        if pulls.len() == 4 {
            pulls
                .join_next()
                .await
                .context("asset download missing")???;
        }
    }
    while let Some(result) = pulls.join_next().await {
        result??;
    }
    Ok(requested)
}
