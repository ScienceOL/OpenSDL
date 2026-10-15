use clap::Subcommand;
use opensdl_assets::{
    daemon::{execute, Operation},
    AssetCache, AssetSummary, PackageDescriptor, PruneReport,
};
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum AssetCmd {
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
