use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use clap::Args;
use opensdl_assets::{
    pack_directory, AssetCache, AssetError, AssetLineage, SourceFormat, VirtualAssetManifest,
};
use serde::Serialize;

#[derive(Debug, Args)]
pub struct SplitArgs {
    /// Directory containing a validated OpenUSD asset.
    pub directory: PathBuf,
    /// Output directory that will contain one child asset directory per part.
    #[arg(long, short)]
    pub output: PathBuf,
    /// Component mapping in the form `component=/Prim/Path`. Repeat it for
    /// every component to extract from the source USD stage.
    #[arg(long = "part", required = true)]
    pub parts: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct SplitResult {
    parent_digest: String,
    children: Vec<SplitChild>,
}

#[derive(Debug, Serialize)]
struct SplitChild {
    name: String,
    path: PathBuf,
    source_path: String,
}

pub fn run(args: SplitArgs, root: Option<PathBuf>) -> anyhow::Result<()> {
    if args.output.exists() {
        bail!("split output already exists: {}", args.output.display());
    }
    let validated = VirtualAssetManifest::load(&args.directory)?;
    if validated.manifest.spec.source.format != SourceFormat::OpenUsd {
        bail!("lab split currently requires an OpenUSD source root");
    }
    let source = validated.paths.source_root.clone();
    let cache = match root {
        Some(root) => AssetCache::open(root)?,
        None => AssetCache::discover()?,
    };
    let parent_digest = package_digest(&cache, &args.directory)?;
    let parts = parse_parts(&args.parts)?;
    let composed = Command::new("usdcat")
        .arg("--flatten")
        .arg(&source)
        .output()
        .context("run usdcat; install OpenUSD tools and ensure usdcat is on PATH")?;
    if !composed.status.success() {
        bail!("usdcat could not open {}", source.display());
    }
    let composed_text = String::from_utf8_lossy(&composed.stdout);
    for (_, prim_path) in &parts {
        let leaf = prim_path.rsplit('/').next().unwrap_or_default();
        if !composed_text.contains(&format!("\"{leaf}\"")) {
            bail!("USD prim does not exist in the composed stage: {prim_path}");
        }
    }
    fs::create_dir_all(&args.output)
        .with_context(|| format!("create split output {}", args.output.display()))?;

    let mut children = Vec::with_capacity(parts.len());
    for (name, prim_path) in parts {
        let child_root = args.output.join(&name);
        fs::create_dir(&child_root)
            .with_context(|| format!("create child asset {}", child_root.display()))?;
        let source_dir = child_root.join("source");
        fs::create_dir_all(&source_dir)?;
        let child_source = source_dir.join(format!("{name}.usda"));
        extract_usd_component(&source, &prim_path, &child_source)
            .with_context(|| format!("extract {prim_path} into {name}"))?;

        copy_declared_file(
            &validated.root,
            &validated.manifest.metadata.license.file,
            &child_root,
        )?;
        copy_declared_file(
            &validated.root,
            &validated.manifest.spec.preview.model,
            &child_root,
        )?;
        copy_declared_file(
            &validated.root,
            &validated.manifest.spec.preview.thumbnail,
            &child_root,
        )?;

        let mut manifest = validated.manifest.clone();
        manifest.metadata.name = format!("{}-{name}", manifest.metadata.name);
        manifest.metadata.display_name = format!("{} / {name}", manifest.metadata.display_name);
        manifest.metadata.description = format!(
            "Component {name} extracted from {}.",
            validated.manifest.metadata.display_name
        );
        manifest.metadata.tags.push("component".to_owned());
        manifest.metadata.lineage = Some(AssetLineage {
            parent: parent_digest.clone(),
            component: name.clone(),
            source_path: prim_path.clone(),
        });
        manifest.spec.source.root = format!("source/{name}.usda");
        manifest.spec.runtime = None;
        manifest.spec.capabilities.retain(|capability| {
            matches!(
                capability,
                opensdl_assets::Capability::Visual | opensdl_assets::Capability::Scene
            )
        });
        let manifest_path = child_root.join("asset.yaml");
        let yaml = serde_yaml::to_string(&manifest).context("serialize split manifest")?;
        fs::write(&manifest_path, yaml)
            .with_context(|| format!("write {}", manifest_path.display()))?;
        VirtualAssetManifest::load(&child_root)
            .map_err(|error| anyhow::anyhow!("validate split child {name}: {error}"))?;
        children.push(SplitChild {
            name,
            path: child_root,
            source_path: prim_path,
        });
    }

    let result = SplitResult {
        parent_digest,
        children,
    };
    if args.json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        println!("parent {}", result.parent_digest);
        for child in &result.children {
            println!("{} {}", child.name, child.path.display());
        }
    }
    Ok(())
}

fn parse_parts(parts: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    let mut parsed = Vec::with_capacity(parts.len());
    for part in parts {
        let (name, prim) = part
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--part must use NAME=/Prim/Path: {part}"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            bail!("invalid component name: {name}");
        }
        if !prim.starts_with('/') || prim == "/" {
            bail!("component prim path must be an absolute non-root USD path: {prim}");
        }
        if parsed.iter().any(|(existing, _)| existing == name) {
            bail!("duplicate component name: {name}");
        }
        parsed.push((name.to_owned(), prim.to_owned()));
    }
    Ok(parsed)
}

fn extract_usd_component(source: &Path, prim_path: &str, output: &Path) -> anyhow::Result<()> {
    let status = Command::new("usdcat")
        .arg("--flatten")
        .arg("--mask")
        .arg(prim_path)
        .arg(source)
        .arg("--out")
        .arg(output)
        .status()
        .context("run usdcat; install OpenUSD tools and ensure usdcat is on PATH")?;
    if !status.success() {
        bail!("usdcat failed for {prim_path} with status {status}");
    }
    Ok(())
}

fn copy_declared_file(
    root: &Path,
    declared: &str,
    destination_root: &Path,
) -> Result<(), AssetError> {
    let relative = Path::new(declared);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(AssetError::Invalid(format!(
            "declared path is not relative: {declared}"
        )));
    }
    let source = root.join(relative);
    let destination = destination_root.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AssetError::Invalid(format!("create {}: {error}", parent.display()))
        })?;
    }
    fs::copy(&source, &destination)
        .map_err(|error| AssetError::Invalid(format!("copy {}: {error}", destination.display())))?;
    Ok(())
}

fn package_digest(cache: &AssetCache, directory: &Path) -> anyhow::Result<String> {
    let temporary = tempfile::tempdir_in(cache.root().join("staging"))?;
    let layout = temporary.path().join("parent-layout");
    let package = pack_directory(directory, &layout)?;
    Ok(package.manifest_digest)
}
