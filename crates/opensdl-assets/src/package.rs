use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Builder, EntryType, Header};
use walkdir::WalkDir;

use crate::manifest::{AssetError, ValidatedAsset, VirtualAssetManifest};
use crate::media_types::{
    ARTIFACT_TYPE, CONFIG_MEDIA_TYPE, INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, PREVIEW_MEDIA_TYPE,
    SOURCE_MEDIA_TYPE,
};
use crate::reference::validate_digest;

const OCI_LAYOUT_VERSION: &str = "1.0.0";
const REF_NAME_ANNOTATION: &str = "org.opencontainers.image.ref.name";
const TITLE_ANNOTATION: &str = "org.opencontainers.image.title";
const VERSION_ANNOTATION: &str = "org.opencontainers.image.version";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OciDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDescriptor {
    pub asset: VirtualAssetManifest,
    pub artifact_type: String,
    pub manifest_media_type: String,
    pub manifest_digest: String,
    pub manifest_size: u64,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImageManifest {
    pub schema_version: u32,
    pub media_type: String,
    pub artifact_type: String,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImageIndex {
    pub schema_version: u32,
    pub media_type: String,
    pub manifests: Vec<OciDescriptor>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OciLayout {
    image_layout_version: String,
}

pub fn pack_directory(source: &Path, output: &Path) -> Result<PackageDescriptor, AssetError> {
    let asset = VirtualAssetManifest::load(source)?;
    if output.exists() {
        return Err(AssetError::invalid(format!(
            "output already exists: {}",
            output.display()
        )));
    }

    let config_bytes = serde_json::to_vec(&asset.manifest)
        .map_err(|error| AssetError::invalid(format!("serialize asset config: {error}")))?;
    let source_bytes = build_source_layer(&asset)?;
    let preview_bytes = read(&asset.paths.preview_model)?;
    let thumbnail_bytes = read(&asset.paths.thumbnail)?;

    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| AssetError::read(parent, source))?;
    let staging = tempfile::Builder::new()
        .prefix(".opensdl-pack-")
        .tempdir_in(parent)
        .map_err(|source| AssetError::read(parent, source))?;
    let layout_root = staging.path().join("layout");
    let blob_root = layout_root.join("blobs/sha256");
    fs::create_dir_all(&blob_root).map_err(|source| AssetError::read(&blob_root, source))?;

    let config = write_blob(&blob_root, CONFIG_MEDIA_TYPE, &config_bytes, None)?;
    let source_layer = write_blob(
        &blob_root,
        SOURCE_MEDIA_TYPE,
        &source_bytes,
        Some("source.tar.zst"),
    )?;
    let preview_layer = write_blob(
        &blob_root,
        PREVIEW_MEDIA_TYPE,
        &preview_bytes,
        Some(&asset.manifest.spec.preview.model),
    )?;
    let thumbnail_media_type = thumbnail_media_type(&asset.paths.thumbnail)?;
    let thumbnail_layer = write_blob(
        &blob_root,
        thumbnail_media_type,
        &thumbnail_bytes,
        Some(&asset.manifest.spec.preview.thumbnail),
    )?;
    let layers = vec![source_layer, preview_layer, thumbnail_layer];

    let manifest = ImageManifest {
        schema_version: 2,
        media_type: MANIFEST_MEDIA_TYPE.to_owned(),
        artifact_type: ARTIFACT_TYPE.to_owned(),
        config: config.clone(),
        layers: layers.clone(),
        annotations: BTreeMap::from([
            (
                TITLE_ANNOTATION.to_owned(),
                asset.manifest.metadata.display_name.clone(),
            ),
            (
                VERSION_ANNOTATION.to_owned(),
                asset.manifest.metadata.version.clone(),
            ),
        ]),
    };
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| AssetError::invalid(format!("serialize OCI manifest: {error}")))?;
    let mut manifest_descriptor =
        write_blob(&blob_root, MANIFEST_MEDIA_TYPE, &manifest_bytes, None)?;
    manifest_descriptor.artifact_type = Some(ARTIFACT_TYPE.to_owned());
    manifest_descriptor.annotations.insert(
        REF_NAME_ANNOTATION.to_owned(),
        asset.manifest.metadata.version.clone(),
    );

    write_json(
        &layout_root.join("oci-layout"),
        &OciLayout {
            image_layout_version: OCI_LAYOUT_VERSION.to_owned(),
        },
    )?;
    write_json(
        &layout_root.join("index.json"),
        &ImageIndex {
            schema_version: 2,
            media_type: INDEX_MEDIA_TYPE.to_owned(),
            manifests: vec![manifest_descriptor.clone()],
        },
    )?;

    fs::rename(&layout_root, output).map_err(|source| AssetError::read(output, source))?;
    Ok(PackageDescriptor {
        asset: asset.manifest,
        artifact_type: ARTIFACT_TYPE.to_owned(),
        manifest_media_type: MANIFEST_MEDIA_TYPE.to_owned(),
        manifest_digest: manifest_descriptor.digest,
        manifest_size: manifest_descriptor.size,
        config,
        layers,
    })
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn blob_path(root: &Path, digest: &str) -> Result<PathBuf, AssetError> {
    Ok(root.join("blobs/sha256").join(validate_digest(digest)?))
}

fn build_source_layer(asset: &ValidatedAsset) -> Result<Vec<u8>, AssetError> {
    let mut files = WalkDir::new(&asset.root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.map_err(|error| AssetError::invalid(format!("walk asset: {error}"))))
        .collect::<Result<Vec<_>, _>>()?;
    files.retain(|entry| {
        entry.file_type().is_file()
            && entry.path() != asset.paths.preview_model
            && entry.path() != asset.paths.thumbnail
    });
    files.sort_by(|left, right| left.path().cmp(right.path()));

    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 10)
        .map_err(|error| AssetError::invalid(format!("create zstd encoder: {error}")))?;
    encoder
        .include_checksum(true)
        .map_err(|error| AssetError::invalid(format!("configure zstd encoder: {error}")))?;
    {
        let mut archive = Builder::new(&mut encoder);
        for entry in files {
            let relative = entry
                .path()
                .strip_prefix(&asset.root)
                .map_err(|_| AssetError::invalid("source path escapes asset root"))?;
            let archive_path = to_posix(relative)?;
            let bytes = read(entry.path())?;
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Regular);
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_cksum();
            archive
                .append_data(&mut header, archive_path, bytes.as_slice())
                .map_err(|error| AssetError::invalid(format!("append source file: {error}")))?;
        }
        archive
            .finish()
            .map_err(|error| AssetError::invalid(format!("finish source archive: {error}")))?;
    }
    encoder
        .finish()
        .map_err(|error| AssetError::invalid(format!("finish zstd stream: {error}")))
}

fn to_posix(path: &Path) -> Result<String, AssetError> {
    let mut parts = Vec::new();
    for component in path.components() {
        let value = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| AssetError::invalid("asset paths must be valid UTF-8"))?;
        parts.push(value);
    }
    Ok(parts.join("/"))
}

fn thumbnail_media_type(path: &Path) -> Result<&'static str, AssetError> {
    match path.extension().and_then(|value| value.to_str()) {
        Some("png") => Ok("image/png"),
        Some("webp") => Ok("image/webp"),
        _ => Err(AssetError::invalid("thumbnail must use .png or .webp")),
    }
}

fn write_blob(
    blob_root: &Path,
    media_type: &str,
    bytes: &[u8],
    title: Option<&str>,
) -> Result<OciDescriptor, AssetError> {
    let digest = digest_bytes(bytes);
    let path = blob_root.join(digest.trim_start_matches("sha256:"));
    let mut file = File::create(&path).map_err(|source| AssetError::read(&path, source))?;
    file.write_all(bytes)
        .map_err(|source| AssetError::read(&path, source))?;
    file.sync_all()
        .map_err(|source| AssetError::read(&path, source))?;
    let annotations = title
        .map(|title| BTreeMap::from([(TITLE_ANNOTATION.to_owned(), title.to_owned())]))
        .unwrap_or_default();
    Ok(OciDescriptor {
        media_type: media_type.to_owned(),
        digest,
        size: bytes.len() as u64,
        artifact_type: None,
        annotations,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), AssetError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| AssetError::invalid(format!("serialize OCI JSON: {error}")))?;
    let mut file = File::create(path).map_err(|source| AssetError::read(path, source))?;
    file.write_all(&bytes)
        .map_err(|source| AssetError::read(path, source))?;
    file.sync_all()
        .map_err(|source| AssetError::read(path, source))
}

fn read(path: &Path) -> Result<Vec<u8>, AssetError> {
    fs::read(path).map_err(|source| AssetError::read(path, source))
}
