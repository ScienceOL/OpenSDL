use std::fs;
use std::path::Path;

use crate::manifest::{AssetError, VirtualAssetManifest};
use crate::media_types::{
    ARTIFACT_TYPE, CONFIG_MEDIA_TYPE, INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, PREVIEW_MEDIA_TYPE,
    SOURCE_MEDIA_TYPE,
};
use crate::package::{
    blob_path, digest_bytes, ImageIndex, ImageManifest, OciDescriptor, PackageDescriptor,
};

pub fn inspect_layout(root: &Path) -> Result<PackageDescriptor, AssetError> {
    let layout: serde_json::Value = read_json(&root.join("oci-layout"))?;
    if layout
        .get("imageLayoutVersion")
        .and_then(|value| value.as_str())
        != Some("1.0.0")
    {
        return Err(AssetError::invalid("unsupported OCI image layout version"));
    }

    let index: ImageIndex = read_json(&root.join("index.json"))?;
    if index.schema_version != 2 || index.media_type != INDEX_MEDIA_TYPE {
        return Err(AssetError::invalid("invalid OCI image index"));
    }
    let [manifest_descriptor] = index.manifests.as_slice() else {
        return Err(AssetError::invalid(
            "OCI layout must contain exactly one manifest",
        ));
    };
    if manifest_descriptor.media_type != MANIFEST_MEDIA_TYPE
        || manifest_descriptor.artifact_type.as_deref() != Some(ARTIFACT_TYPE)
    {
        return Err(AssetError::invalid(
            "OCI index does not describe an OpenSDL asset",
        ));
    }
    let manifest_bytes = verify_blob(root, manifest_descriptor)?;
    let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| AssetError::invalid(format!("parse OCI manifest: {error}")))?;
    validate_manifest_shape(&manifest)?;

    let config_bytes = verify_blob(root, &manifest.config)?;
    let asset = serde_json::from_slice::<VirtualAssetManifest>(&config_bytes)
        .map_err(|error| AssetError::invalid(format!("parse OpenSDL config: {error}")))?;
    asset.validate_declaration()?;
    for layer in &manifest.layers {
        verify_blob(root, layer)?;
    }

    Ok(PackageDescriptor {
        asset,
        artifact_type: manifest.artifact_type,
        manifest_media_type: manifest.media_type,
        manifest_digest: manifest_descriptor.digest.clone(),
        manifest_size: manifest_descriptor.size,
        config: manifest.config,
        layers: manifest.layers,
    })
}

pub(crate) fn verify_blob(root: &Path, descriptor: &OciDescriptor) -> Result<Vec<u8>, AssetError> {
    let path = blob_path(root, &descriptor.digest)?;
    let bytes = fs::read(&path).map_err(|source| AssetError::read(&path, source))?;
    if bytes.len() as u64 != descriptor.size {
        return Err(AssetError::invalid(format!(
            "blob size mismatch for {}",
            descriptor.digest
        )));
    }
    let actual = digest_bytes(&bytes);
    if actual != descriptor.digest {
        return Err(AssetError::invalid(format!(
            "blob digest mismatch: expected {}, got {actual}",
            descriptor.digest
        )));
    }
    Ok(bytes)
}

pub(crate) fn validate_manifest_shape(manifest: &ImageManifest) -> Result<(), AssetError> {
    if manifest.schema_version != 2
        || manifest.media_type != MANIFEST_MEDIA_TYPE
        || manifest.artifact_type != ARTIFACT_TYPE
        || manifest.config.media_type != CONFIG_MEDIA_TYPE
    {
        return Err(AssetError::invalid("invalid OpenSDL OCI manifest"));
    }
    let [source, preview, thumbnail] = manifest.layers.as_slice() else {
        return Err(AssetError::invalid(
            "OpenSDL OCI manifest must contain source, preview, and thumbnail layers",
        ));
    };
    if source.media_type != SOURCE_MEDIA_TYPE || preview.media_type != PREVIEW_MEDIA_TYPE {
        return Err(AssetError::invalid(
            "invalid OpenSDL asset layer media types",
        ));
    }
    if thumbnail.media_type != "image/png" && thumbnail.media_type != "image/webp" {
        return Err(AssetError::invalid("invalid OpenSDL thumbnail media type"));
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, AssetError> {
    let bytes = fs::read(path).map_err(|source| AssetError::read(path, source))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AssetError::invalid(format!("parse {}: {error}", path.display())))
}
