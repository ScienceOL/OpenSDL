use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::layout::validate_manifest_shape;
use crate::manifest::{AssetError, VirtualAssetManifest};
use crate::media_types::MANIFEST_MEDIA_TYPE;
use crate::package::{ImageManifest, OciDescriptor, PackageDescriptor};
use crate::reference::{validate_digest, AssetReference};

#[derive(Debug, Clone)]
pub struct AssetCache {
    root: PathBuf,
    blobs: PathBuf,
    refs: PathBuf,
    quarantine: PathBuf,
    staging: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefRecord {
    reference: String,
    manifest_digest: String,
}

impl AssetCache {
    pub fn discover() -> Result<Self, AssetError> {
        let dirs = ProjectDirs::from("com", "scienceol", "osdl")
            .ok_or_else(|| AssetError::invalid("no valid user cache directory"))?;
        Self::open(dirs.cache_dir().join("assets/v1"))
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self, AssetError> {
        let root = root.as_ref().to_path_buf();
        let cache = Self {
            blobs: root.join("blobs/sha256"),
            refs: root.join("refs"),
            quarantine: root.join("quarantine"),
            staging: root.join("staging"),
            root,
        };
        for directory in [
            &cache.root,
            &cache.blobs,
            &cache.refs,
            &cache.quarantine,
            &cache.staging,
        ] {
            fs::create_dir_all(directory).map_err(|source| AssetError::read(directory, source))?;
        }
        Ok(cache)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn quarantine_dir(&self) -> &Path {
        &self.quarantine
    }

    pub fn blob_path(&self, digest: &str) -> Result<PathBuf, AssetError> {
        Ok(self.blobs.join(validate_digest(digest)?))
    }

    pub fn contains(&self, descriptor: &OciDescriptor) -> Result<bool, AssetError> {
        let path = self.blob_path(&descriptor.digest)?;
        if !path.exists() {
            return Ok(false);
        }
        if verify_path(&path, &descriptor.digest, Some(descriptor.size)).is_ok() {
            return Ok(true);
        }
        self.quarantine(&path, &descriptor.digest)?;
        Ok(false)
    }

    pub fn store_bytes(&self, descriptor: &OciDescriptor, bytes: &[u8]) -> Result<(), AssetError> {
        verify_bytes(bytes, &descriptor.digest, Some(descriptor.size))?;
        if self.contains(descriptor)? {
            return Ok(());
        }
        let mut temporary = tempfile::Builder::new()
            .prefix("blob-")
            .tempfile_in(&self.staging)
            .map_err(|source| AssetError::read(&self.staging, source))?;
        temporary
            .write_all(bytes)
            .map_err(|source| AssetError::read(temporary.path(), source))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|source| AssetError::read(temporary.path(), source))?;
        self.persist(descriptor, temporary)
    }

    pub(crate) fn persist(
        &self,
        descriptor: &OciDescriptor,
        temporary: tempfile::NamedTempFile,
    ) -> Result<(), AssetError> {
        verify_path(temporary.path(), &descriptor.digest, Some(descriptor.size))?;
        let destination = self.blob_path(&descriptor.digest)?;
        match temporary.persist_noclobber(&destination) {
            Ok(file) => file
                .sync_all()
                .map_err(|source| AssetError::read(&destination, source)),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                if self.contains(descriptor)? {
                    Ok(())
                } else {
                    Err(AssetError::read(&destination, error.error))
                }
            }
            Err(error) => Err(AssetError::read(&destination, error.error)),
        }
    }

    pub(crate) fn temporary_blob(&self) -> Result<tempfile::NamedTempFile, AssetError> {
        tempfile::Builder::new()
            .prefix("download-")
            .tempfile_in(&self.staging)
            .map_err(|source| AssetError::read(&self.staging, source))
    }

    pub fn import_layout(&self, layout: &Path) -> Result<PackageDescriptor, AssetError> {
        let package = crate::inspect_layout(layout)?;
        let manifest_descriptor = OciDescriptor {
            media_type: MANIFEST_MEDIA_TYPE.to_owned(),
            digest: package.manifest_digest.clone(),
            size: package.manifest_size,
            artifact_type: Some(package.artifact_type.clone()),
            annotations: Default::default(),
        };
        for descriptor in std::iter::once(&manifest_descriptor)
            .chain(std::iter::once(&package.config))
            .chain(package.layers.iter())
        {
            let source = layout
                .join("blobs/sha256")
                .join(validate_digest(&descriptor.digest)?);
            let bytes = fs::read(&source).map_err(|error| AssetError::read(&source, error))?;
            self.store_bytes(descriptor, &bytes)?;
        }
        Ok(package)
    }

    pub fn package(&self, manifest_digest: &str) -> Result<PackageDescriptor, AssetError> {
        let bytes = self.read_digest(manifest_digest)?;
        let manifest: ImageManifest = serde_json::from_slice(&bytes)
            .map_err(|error| AssetError::invalid(format!("parse cached OCI manifest: {error}")))?;
        validate_manifest_shape(&manifest)?;
        if !self.contains(&manifest.config)? {
            return Err(AssetError::invalid(format!(
                "cached config blob is missing: {}",
                manifest.config.digest
            )));
        }
        for layer in &manifest.layers {
            if !self.contains(layer)? {
                return Err(AssetError::invalid(format!(
                    "cached layer blob is missing: {}",
                    layer.digest
                )));
            }
        }
        let asset: VirtualAssetManifest =
            serde_json::from_slice(&self.read_descriptor(&manifest.config)?).map_err(|error| {
                AssetError::invalid(format!("parse cached asset config: {error}"))
            })?;
        asset.validate_declaration()?;
        Ok(PackageDescriptor {
            asset,
            artifact_type: manifest.artifact_type,
            manifest_media_type: manifest.media_type,
            manifest_digest: manifest_digest.to_owned(),
            manifest_size: bytes.len() as u64,
            config: manifest.config,
            layers: manifest.layers,
        })
    }

    pub(crate) fn read_descriptor(
        &self,
        descriptor: &OciDescriptor,
    ) -> Result<Vec<u8>, AssetError> {
        if !self.contains(descriptor)? {
            return Err(AssetError::invalid(format!(
                "cached blob is missing: {}",
                descriptor.digest
            )));
        }
        let path = self.blob_path(&descriptor.digest)?;
        fs::read(&path).map_err(|source| AssetError::read(&path, source))
    }

    pub(crate) fn read_digest(&self, digest: &str) -> Result<Vec<u8>, AssetError> {
        let path = self.blob_path(digest)?;
        let bytes = fs::read(&path).map_err(|source| AssetError::read(&path, source))?;
        verify_bytes(&bytes, digest, None)?;
        Ok(bytes)
    }

    pub fn record_reference(
        &self,
        reference: &AssetReference,
        manifest_digest: &str,
    ) -> Result<(), AssetError> {
        validate_digest(manifest_digest)?;
        let path = self.reference_path(reference);
        let record = RefRecord {
            reference: reference.to_string(),
            manifest_digest: manifest_digest.to_owned(),
        };
        let bytes = serde_json::to_vec(&record)
            .map_err(|error| AssetError::invalid(format!("serialize ref hint: {error}")))?;
        let mut temporary = tempfile::Builder::new()
            .prefix("ref-")
            .tempfile_in(&self.staging)
            .map_err(|source| AssetError::read(&self.staging, source))?;
        temporary
            .write_all(&bytes)
            .map_err(|source| AssetError::read(temporary.path(), source))?;
        temporary
            .persist(&path)
            .map_err(|error| AssetError::read(&path, error.error))?;
        Ok(())
    }

    pub fn resolve_reference(&self, reference: &AssetReference) -> Result<String, AssetError> {
        if let Some(digest) = reference.digest() {
            return Ok(digest.to_owned());
        }
        let path = self.reference_path(reference);
        let bytes = fs::read(&path).map_err(|source| AssetError::read(&path, source))?;
        let record: RefRecord = serde_json::from_slice(&bytes)
            .map_err(|error| AssetError::invalid(format!("parse ref hint: {error}")))?;
        if record.reference != reference.to_string() {
            return Err(AssetError::invalid("cached ref hint identity mismatch"));
        }
        validate_digest(&record.manifest_digest)?;
        Ok(record.manifest_digest)
    }

    fn reference_path(&self, reference: &AssetReference) -> PathBuf {
        let key = Sha256::digest(reference.to_string().as_bytes());
        self.refs.join(format!("{key:x}.json"))
    }

    fn quarantine(&self, path: &Path, digest: &str) -> Result<(), AssetError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let hash = validate_digest(digest)?;
        let destination = self.quarantine.join(format!("{hash}.{timestamp}.bad"));
        fs::rename(path, &destination).map_err(|source| AssetError::read(path, source))
    }
}

fn verify_path(path: &Path, digest: &str, size: Option<u64>) -> Result<(), AssetError> {
    let metadata = fs::metadata(path).map_err(|source| AssetError::read(path, source))?;
    if size.is_some_and(|expected| expected != metadata.len()) {
        return Err(AssetError::invalid(format!(
            "blob size mismatch for {digest}"
        )));
    }
    let mut file = File::open(path).map_err(|source| AssetError::read(path, source))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| AssetError::read(path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("sha256:{:x}", hasher.finalize());
    if actual != digest {
        return Err(AssetError::invalid(format!(
            "blob digest mismatch: expected {digest}, got {actual}"
        )));
    }
    Ok(())
}

fn verify_bytes(bytes: &[u8], digest: &str, size: Option<u64>) -> Result<(), AssetError> {
    if size.is_some_and(|expected| expected != bytes.len() as u64) {
        return Err(AssetError::invalid(format!(
            "blob size mismatch for {digest}"
        )));
    }
    let actual = format!("sha256:{:x}", Sha256::digest(bytes));
    if actual != digest {
        return Err(AssetError::invalid(format!(
            "blob digest mismatch: expected {digest}, got {actual}"
        )));
    }
    Ok(())
}
