use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use clap::Args;
use opensdl_assets::{pack_directory, AssetLineage, SourceFormat, VirtualAssetManifest};
use serde::Serialize;

#[derive(Debug, Args)]
pub struct SplitArgs {
    /// Directory containing a validated OpenUSD asset.
    pub directory: PathBuf,
    /// New directory that will contain one child asset directory per part.
    #[arg(long, short)]
    pub output: PathBuf,
    /// Semantic component mapping NAME=/Prim/Path; repeat for each component.
    #[arg(long = "part", required = true)]
    pub parts: Vec<String>,
    /// Python executable with the OpenUSD pxr package (usd-core==25.11).
    #[arg(long, env = "LAB_USD_PYTHON", default_value = "python3")]
    pub python: PathBuf,
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

pub fn run(args: SplitArgs) -> anyhow::Result<()> {
    if args.output.exists() {
        bail!("split output already exists: {}", args.output.display());
    }
    let validated = VirtualAssetManifest::load(&args.directory)?;
    if validated.manifest.spec.source.format != SourceFormat::OpenUsd {
        bail!("lab split requires an OpenUSD source root");
    }
    let parts = parse_parts(&args.parts)?;
    let output = std::path::absolute(&args.output)?;
    let parent = output
        .parent()
        .context("split output needs a parent directory")?;
    fs::create_dir_all(parent)?;
    // Publish all children together. A failed extraction leaves the target absent and retryable.
    let staging = tempfile::tempdir_in(parent)?;
    let layout = tempfile::tempdir()?;
    let parent_digest =
        pack_directory(&args.directory, &layout.path().join("parent"))?.manifest_digest;
    let manifests = parts
        .iter()
        .map(|(name, prim)| child_manifest(&validated.manifest, &parent_digest, name, prim))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let extraction = Command::new(&args.python)
        .args(["-c", include_str!("split/extract.py")])
        .arg(&validated.paths.source_root)
        .arg(&validated.root)
        .arg(staging.path())
        .arg(serde_json::to_string(&parts)?)
        .output()
        .context(
            "run OpenUSD Python; install usd-core==25.11 and set LAB_USD_PYTHON or --python",
        )?;
    if !extraction.status.success() {
        bail!(
            "OpenUSD extraction failed: {}",
            String::from_utf8_lossy(&extraction.stderr).trim()
        );
    }
    let mut children = Vec::with_capacity(parts.len());
    for ((name, prim), manifest) in parts.into_iter().zip(manifests) {
        let child_root = staging.path().join(&name);
        for declared in [
            &manifest.metadata.license.file,
            &manifest.spec.preview.model,
            &manifest.spec.preview.thumbnail,
        ] {
            copy_declared_file(&validated.root, declared, &child_root)?;
        }
        fs::write(
            child_root.join("asset.yaml"),
            serde_yaml::to_string(&manifest)?,
        )?;
        VirtualAssetManifest::load(&child_root)
            .with_context(|| format!("validate split child {name}"))?;
        children.push(SplitChild {
            path: output.join(&name),
            name,
            source_path: prim,
        });
    }
    // rename is on the same filesystem as the destination.
    fs::rename(staging.path(), &output).context("publish split children")?;
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

fn child_manifest(
    parent: &VirtualAssetManifest,
    digest: &str,
    name: &str,
    prim: &str,
) -> anyhow::Result<VirtualAssetManifest> {
    let mut manifest = parent.clone();
    manifest.metadata.name = format!("{}-{name}", parent.metadata.name);
    manifest.metadata.display_name = format!("{} / {name}", parent.metadata.display_name);
    manifest.metadata.description = format!(
        "Component {name} extracted from {}.",
        parent.metadata.display_name
    );
    if !manifest.metadata.tags.iter().any(|tag| tag == "component") {
        manifest.metadata.tags.push("component".into());
    }
    manifest.metadata.lineage = Some(AssetLineage {
        parent: digest.into(),
        component: name.into(),
        source_path: prim.into(),
    });
    manifest.spec.source.root = format!("source/{name}.usda");
    manifest.spec.runtime = None;
    manifest.spec.bindings = Default::default();
    manifest.spec.compatibility = Default::default();
    manifest.spec.capabilities.retain(|capability| {
        matches!(
            capability,
            opensdl_assets::Capability::Visual | opensdl_assets::Capability::Scene
        )
    });
    manifest.validate_declaration()?;
    Ok(manifest)
}

fn parse_parts(parts: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    let mut parsed = Vec::with_capacity(parts.len());
    for part in parts {
        let (name, prim) = part
            .split_once('=')
            .context("--part must use NAME=/Prim/Path")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || name.starts_with('-')
            || name.ends_with('-')
        {
            bail!(
                "component name must use lowercase letters, digits, and interior hyphens: {name}"
            );
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

fn copy_declared_file(root: &Path, declared: &str, destination: &Path) -> anyhow::Result<()> {
    // Parent manifest validation has already proved these are portable relative paths.
    let target = destination.join(declared);
    if target.exists() {
        bail!("preview/license path conflicts with extracted source: {declared}");
    }
    fs::create_dir_all(target.parent().context("declared file has no parent")?)?;
    fs::copy(root.join(declared), &target)
        .with_context(|| format!("copy declared file {declared}"))?;
    Ok(())
}
