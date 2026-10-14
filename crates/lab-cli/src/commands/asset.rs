use clap::Subcommand;
use opensdl_assets::{
    daemon::DaemonClient, AssetCache, AssetReference, AssetSummary, PackageDescriptor, PruneReport,
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
    if let Some(client) = DaemonClient::discover(&cache).await? {
        match &command {
            AssetCmd::List { json } => {
                let assets: Vec<AssetSummary> =
                    client.call_json("list", serde_json::json!({})).await?;
                if *json {
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
                let package: PackageDescriptor = client
                    .call_json(
                        "import",
                        serde_json::json!({
                            "layout": layout,
                            "tag": tag,
                        }),
                    )
                    .await?;
                println!("{}", package.manifest_digest);
            }
            AssetCmd::Tag { source, target } => {
                client
                    .call(
                        "tag",
                        serde_json::json!({"source": source, "target": target}),
                    )
                    .await?;
            }
            AssetCmd::Inspect { reference, json } => {
                let package: PackageDescriptor = client
                    .call_json("inspect", serde_json::json!({"reference": reference}))
                    .await?;
                if *json {
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
                client
                    .call("remove", serde_json::json!({"reference": reference}))
                    .await?;
            }
            AssetCmd::Prune { dry_run, json } => {
                let report: PruneReport = client
                    .call_json("prune", serde_json::json!({"dryRun": dry_run}))
                    .await?;
                if *json {
                    println!("{}", serde_json::to_string(&report)?);
                } else {
                    println!(
                        "{} packages, {} blobs, {} bytes{}",
                        report.packages.len(),
                        report.blobs.len(),
                        report.reclaimed_bytes,
                        if *dry_run { " (dry run)" } else { " reclaimed" }
                    );
                }
            }
        }
        return Ok(());
    }
    match command {
        AssetCmd::List { json } => {
            let assets = cache.list()?;
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
            let reference = tag.as_deref().map(AssetReference::parse).transpose()?;
            let package = cache.lock()?.import_layout(&layout, reference.as_ref())?;
            println!("{}", package.manifest_digest);
        }
        AssetCmd::Tag { source, target } => cache.tag(&source, &AssetReference::parse(&target)?)?,
        AssetCmd::Inspect { reference, json } => {
            let package = cache.inspect(&reference)?;
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
        AssetCmd::Remove { reference } => cache.remove(&reference)?,
        AssetCmd::Prune { dry_run, json } => {
            let report = cache.prune(dry_run)?;
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
