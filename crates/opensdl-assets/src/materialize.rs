use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use tar::EntryType;

use crate::cache::AssetCache;
use crate::manifest::{AssetError, ValidatedAsset, VirtualAssetManifest};
use crate::media_types::{PREVIEW_MEDIA_TYPE, SOURCE_MEDIA_TYPE};
use crate::package::OciDescriptor;
use crate::paths::safe_relative_path;

const MAX_EXPANDED_FILES: usize = 100_000;
const MAX_EXPANDED_BYTES: u64 = 8 * 1024 * 1024 * 1024;

pub fn materialize(
    cache: &AssetCache,
    manifest_digest: &str,
    destination: &Path,
) -> Result<ValidatedAsset, AssetError> {
    if destination.exists() {
        return Err(AssetError::invalid(format!(
            "destination already exists: {}",
            destination.display()
        )));
    }

    let package = cache.package(manifest_digest)?;
    let manifest: VirtualAssetManifest =
        serde_json::from_slice(&cache.read_descriptor(&package.config)?)
            .map_err(|error| AssetError::invalid(format!("parse cached asset config: {error}")))?;
    let [source, preview, thumbnail] = package.layers.as_slice() else {
        return Err(AssetError::invalid(
            "OpenSDL OCI manifest must contain source, preview, and thumbnail layers",
        ));
    };
    if source.media_type != SOURCE_MEDIA_TYPE || preview.media_type != PREVIEW_MEDIA_TYPE {
        return Err(AssetError::invalid(
            "invalid OpenSDL asset layer media types",
        ));
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| AssetError::read(parent, source))?;
    let staging = tempfile::Builder::new()
        .prefix(".opensdl-materialize-")
        .tempdir_in(parent)
        .map_err(|source| AssetError::read(parent, source))?;
    let root = staging.path().join("asset");
    fs::create_dir(&root).map_err(|source| AssetError::read(&root, source))?;

    extract_source(cache, source, &root)?;
    write_declared_layer(
        cache,
        preview,
        &root,
        &manifest.spec.preview.model,
        "preview model",
    )?;
    write_declared_layer(
        cache,
        thumbnail,
        &root,
        &manifest.spec.preview.thumbnail,
        "thumbnail",
    )?;

    let manifest_path = root.join("asset.yaml");
    let yaml_manifest: VirtualAssetManifest = serde_yaml::from_slice(
        &fs::read(&manifest_path).map_err(|source| AssetError::read(&manifest_path, source))?,
    )?;
    if yaml_manifest != manifest {
        return Err(AssetError::invalid(
            "asset.yaml does not match the immutable OCI config",
        ));
    }
    VirtualAssetManifest::load(&root)?;

    fs::rename(&root, destination).map_err(|source| AssetError::read(destination, source))?;
    VirtualAssetManifest::load(destination)
}

fn extract_source(
    cache: &AssetCache,
    descriptor: &OciDescriptor,
    destination: &Path,
) -> Result<(), AssetError> {
    let source_path = cache.blob_path(&descriptor.digest)?;
    let file =
        fs::File::open(&source_path).map_err(|source| AssetError::read(&source_path, source))?;
    let decoder = zstd::stream::read::Decoder::new(file)
        .map_err(|error| AssetError::invalid(format!("open source layer: {error}")))?;
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| AssetError::invalid(format!("read source archive: {error}")))?;
    let mut file_count = 0_usize;
    let mut expanded_bytes = 0_u64;

    for entry in entries {
        let mut entry =
            entry.map_err(|error| AssetError::invalid(format!("read source entry: {error}")))?;
        if entry.header().entry_type() != EntryType::Regular {
            return Err(AssetError::invalid(
                "source archive may contain only regular files",
            ));
        }
        file_count = file_count
            .checked_add(1)
            .ok_or_else(|| AssetError::invalid("source archive file count overflow"))?;
        if file_count > MAX_EXPANDED_FILES {
            return Err(AssetError::invalid(
                "source archive contains too many files",
            ));
        }
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| AssetError::invalid("source archive size overflow"))?;
        if expanded_bytes > MAX_EXPANDED_BYTES {
            return Err(AssetError::invalid("source archive expands beyond 8 GiB"));
        }

        let entry_path = entry
            .path()
            .map_err(|error| AssetError::invalid(format!("read archive path: {error}")))?;
        let entry_path = entry_path
            .to_str()
            .ok_or_else(|| AssetError::invalid("archive paths must be valid UTF-8"))?
            .to_owned();
        let relative = safe_relative_path(&entry_path, "archive")?;
        let output = destination.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|source| AssetError::read(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|source| AssetError::read(&output, source))?;
        let expected_size = entry.size();
        let copied = std::io::copy(&mut entry, &mut file)
            .map_err(|source| AssetError::read(&output, source))?;
        if copied != expected_size {
            return Err(AssetError::invalid(format!(
                "source entry size mismatch: {entry_path}"
            )));
        }
        file.sync_all()
            .map_err(|source| AssetError::read(&output, source))?;
    }
    Ok(())
}

fn write_declared_layer(
    cache: &AssetCache,
    descriptor: &OciDescriptor,
    root: &Path,
    declared_path: &str,
    label: &str,
) -> Result<(), AssetError> {
    let relative = safe_relative_path(declared_path, label)?;
    let destination = root.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| AssetError::read(parent, source))?;
    }
    let bytes = cache.read_descriptor(descriptor)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|source| AssetError::read(&destination, source))?;
    file.write_all(&bytes)
        .map_err(|source| AssetError::read(&destination, source))?;
    file.sync_all()
        .map_err(|source| AssetError::read(&destination, source))
}
