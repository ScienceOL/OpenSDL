use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::manifest::{AssetError, SourceFormat, VirtualAssetManifest};

#[derive(Debug)]
pub struct ValidatedPaths {
    pub manifest: PathBuf,
    pub license: PathBuf,
    pub source_root: PathBuf,
    pub preview_model: PathBuf,
    pub thumbnail: PathBuf,
}

impl ValidatedPaths {
    pub(crate) fn resolve(
        root: &Path,
        manifest: &VirtualAssetManifest,
    ) -> Result<Self, AssetError> {
        let paths = Self {
            manifest: root.join("asset.yaml"),
            license: resolve_declared_file(root, &manifest.metadata.license.file, "license")?,
            source_root: resolve_declared_file(root, &manifest.spec.source.root, "source root")?,
            preview_model: resolve_declared_file(
                root,
                &manifest.spec.preview.model,
                "preview model",
            )?,
            thumbnail: resolve_declared_file(root, &manifest.spec.preview.thumbnail, "thumbnail")?,
        };

        validate_non_empty_license(&paths.license)?;
        validate_source(manifest.spec.source.format, &paths.source_root)?;
        validate_glb(&paths.preview_model, "preview model")?;
        validate_thumbnail(&paths.thumbnail)?;
        if manifest.spec.source.format == SourceFormat::Gltf
            && extension(&paths.source_root) == Some("glb")
            && same_content(&paths.source_root, &paths.preview_model)?
        {
            return Err(AssetError::invalid(
                "preview model must not contain the same bytes as the GLB source root",
            ));
        }
        Ok(paths)
    }
}

pub(crate) fn validate_asset_tree(root: &Path) -> Result<PathBuf, AssetError> {
    let root_metadata =
        fs::symlink_metadata(root).map_err(|source| AssetError::read(root, source))?;
    if root_metadata.file_type().is_symlink() {
        return Err(AssetError::invalid(
            "asset root must not be a symbolic link",
        ));
    }
    if !root_metadata.is_dir() {
        return Err(AssetError::invalid("asset root must be a directory"));
    }
    let root = fs::canonicalize(root).map_err(|source| AssetError::read(root, source))?;
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry =
            entry.map_err(|error| AssetError::invalid(format!("walk asset root: {error}")))?;
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(AssetError::invalid(format!(
                "symbolic link is not allowed: {}",
                entry.path().display()
            )));
        }
        if !(file_type.is_dir() || file_type.is_file()) {
            return Err(AssetError::invalid(format!(
                "special file is not allowed: {}",
                entry.path().display()
            )));
        }
        if file_type.is_file() {
            let metadata = entry
                .metadata()
                .map_err(|error| AssetError::invalid(format!("read metadata: {error}")))?;
            if has_multiple_hard_links(&metadata) {
                return Err(AssetError::invalid(format!(
                    "hard link is not allowed: {}",
                    entry.path().display()
                )));
            }
        }
    }
    Ok(root)
}

pub(crate) fn validate_gltf_dependencies(
    _root: &Path,
    manifest: &VirtualAssetManifest,
    paths: &ValidatedPaths,
) -> Result<(), AssetError> {
    if manifest.spec.source.format != SourceFormat::Gltf
        || extension(&paths.source_root) != Some("gltf")
    {
        return Ok(());
    }
    let bytes = fs::read(&paths.source_root)
        .map_err(|source| AssetError::read(&paths.source_root, source))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| AssetError::invalid(format!("invalid glTF JSON: {error}")))?;
    let source_dir = paths
        .source_root
        .parent()
        .ok_or_else(|| AssetError::invalid("glTF source has no parent directory"))?;
    for section in ["buffers", "images"] {
        let Some(entries) = document.get(section).and_then(Value::as_array) else {
            continue;
        };
        for uri in entries
            .iter()
            .filter_map(|entry| entry.get("uri"))
            .filter_map(Value::as_str)
        {
            if uri.starts_with("data:") {
                continue;
            }
            resolve_declared_file(source_dir, uri, "glTF dependency")?;
        }
    }
    Ok(())
}

fn resolve_declared_file(root: &Path, declared: &str, label: &str) -> Result<PathBuf, AssetError> {
    let relative = safe_relative_path(declared, label)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|source| AssetError::read(&path, source))?;
    if !metadata.is_file() {
        return Err(AssetError::invalid(format!(
            "{label} must be a regular file"
        )));
    }
    let canonical = fs::canonicalize(&path).map_err(|source| AssetError::read(&path, source))?;
    if !canonical.starts_with(root) {
        return Err(AssetError::invalid(format!(
            "{label} path escapes asset root"
        )));
    }
    Ok(canonical)
}

pub(crate) fn safe_relative_path(declared: &str, label: &str) -> Result<PathBuf, AssetError> {
    if declared.is_empty() || declared.contains('\\') || declared.contains(':') {
        return Err(AssetError::invalid(format!(
            "{label} path escapes asset root or is not a relative POSIX path"
        )));
    }
    let relative = Path::new(declared);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AssetError::invalid(format!(
            "{label} path escapes asset root or is not a relative POSIX path"
        )));
    }
    Ok(relative.to_path_buf())
}

fn validate_non_empty_license(path: &Path) -> Result<(), AssetError> {
    let metadata = fs::metadata(path).map_err(|source| AssetError::read(path, source))?;
    if metadata.len() == 0 {
        return Err(AssetError::invalid(
            "bundled license text must not be empty",
        ));
    }
    Ok(())
}

fn validate_source(format: SourceFormat, path: &Path) -> Result<(), AssetError> {
    match (format, extension(path)) {
        (SourceFormat::Gltf, Some("glb")) => validate_glb(path, "source root"),
        (SourceFormat::Gltf, Some("gltf")) => Ok(()),
        (SourceFormat::OpenUsd, Some("usd" | "usda" | "usdc" | "usdz")) => Ok(()),
        (SourceFormat::Gltf, _) => Err(AssetError::invalid(
            "gltf source root must use .glb or .gltf",
        )),
        (SourceFormat::OpenUsd, _) => Err(AssetError::invalid(
            "openusd source root must use .usd, .usda, .usdc, or .usdz",
        )),
    }
}

fn validate_glb(path: &Path, label: &str) -> Result<(), AssetError> {
    let mut file = File::open(path).map_err(|source| AssetError::read(path, source))?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header)
        .map_err(|source| AssetError::read(path, source))?;
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if &header[..4] != b"glTF" || version != 2 {
        return Err(AssetError::invalid(format!(
            "{label} must be a GLB version 2 file"
        )));
    }
    let declared_length = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as u64;
    let actual_length = file
        .seek(SeekFrom::End(0))
        .map_err(|source| AssetError::read(path, source))?;
    if declared_length != actual_length {
        return Err(AssetError::invalid(format!(
            "{label} GLB length does not match its header"
        )));
    }
    Ok(())
}

fn validate_thumbnail(path: &Path) -> Result<(), AssetError> {
    match extension(path) {
        Some("png" | "webp") => Ok(()),
        _ => Err(AssetError::invalid("thumbnail must use .png or .webp")),
    }
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|value| value.to_str())
}

fn same_content(left: &Path, right: &Path) -> Result<bool, AssetError> {
    let left_metadata = fs::metadata(left).map_err(|source| AssetError::read(left, source))?;
    let right_metadata = fs::metadata(right).map_err(|source| AssetError::read(right, source))?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }
    Ok(hash_file(left)? == hash_file(right)?)
}

fn hash_file(path: &Path) -> Result<[u8; 32], AssetError> {
    let mut file = File::open(path).map_err(|source| AssetError::read(path, source))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|source| AssetError::read(path, source))?;
    Ok(hasher.finalize().into())
}

#[cfg(unix)]
fn has_multiple_hard_links(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(windows)]
fn has_multiple_hard_links(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.number_of_links() > 1
}

#[cfg(not(any(unix, windows)))]
fn has_multiple_hard_links(_metadata: &fs::Metadata) -> bool {
    false
}
