use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use sha2::{Digest, Sha256};

use crate::catalog::{AssetSummary, Catalog, PruneReport};
use crate::layout::validate_manifest_shape;
use crate::manifest::{AssetError, VirtualAssetManifest};
use crate::media_types::MANIFEST_MEDIA_TYPE;
use crate::package::{ImageManifest, OciDescriptor, PackageDescriptor};
use crate::reference::{validate_digest, AssetReference};

#[derive(Debug, Clone)]
pub struct AssetCache {
    root: PathBuf,
    blobs: PathBuf,
    quarantine: PathBuf,
    staging: PathBuf,
}

/// Holds the cross-process artifact lock through mutation or materialization.
pub struct CacheLease {
    cache: AssetCache,
    _lock: File,
}

impl AssetCache {
    pub fn discover() -> Result<Self, AssetError> {
        let dirs = ProjectDirs::from("com", "scienceol", "osdl")
            .ok_or_else(|| AssetError::invalid("no valid user cache directory"))?;
        Self::open(
            dirs.state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .join("resources/v1"),
        )
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self, AssetError> {
        ensure_owned_directory(root.as_ref())?;
        let root =
            fs::canonicalize(root.as_ref()).map_err(|e| AssetError::read(root.as_ref(), e))?;
        let cache = Self {
            blobs: root.join("blobs/sha256"),
            quarantine: root.join("quarantine"),
            staging: root.join("staging"),
            root,
        };
        for directory in [
            &cache.root.join("blobs"),
            &cache.blobs,
            &cache.quarantine,
            &cache.staging,
        ] {
            ensure_owned_directory(directory)?;
        }
        let _lease = cache.lock()?;
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
        self.lock()?.store_bytes(descriptor, bytes)
    }

    fn store_bytes_unlocked(
        &self,
        descriptor: &OciDescriptor,
        bytes: &[u8],
    ) -> Result<(), AssetError> {
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

    pub fn lock(&self) -> Result<CacheLease, AssetError> {
        let path = self.root.join("artifact.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| AssetError::read(&path, e))?;
        file.lock().map_err(|e| AssetError::read(&path, e))?;
        Catalog::open(&self.catalog_path())?;
        Ok(CacheLease {
            cache: self.clone(),
            _lock: file,
        })
    }

    pub fn catalog_path(&self) -> PathBuf {
        self.root.join("catalog.sqlite")
    }

    pub fn import_layout(&self, layout: &Path) -> Result<PackageDescriptor, AssetError> {
        self.lock()?.import_layout(layout, None)
    }

    pub async fn lock_async(&self) -> Result<CacheLease, AssetError> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || cache.lock())
            .await
            .map_err(|e| AssetError::invalid(format!("artifact lock task: {e}")))?
    }

    pub fn list(&self) -> Result<Vec<AssetSummary>, AssetError> {
        self.lock()?.catalog()?.list()
    }

    pub fn resolve(&self, selector: &str) -> Result<String, AssetError> {
        let digest = if selector.starts_with("sha256:") {
            crate::reference::validate_digest(selector)?;
            selector.to_owned()
        } else {
            self.resolve_reference(&AssetReference::parse(selector)?)?
        };
        self.package(&digest)?;
        Ok(digest)
    }

    pub fn tag(&self, selector: &str, target: &AssetReference) -> Result<(), AssetError> {
        let lease = self.lock()?;
        target.require_tag()?;
        let digest = self.resolve(selector)?;
        lease.publish(&digest, Some(target)).map(|_| ())
    }

    pub fn remove(&self, selector: &str) -> Result<(), AssetError> {
        self.lock()?.catalog()?.remove(selector)
    }
    pub fn prune(&self, dry_run: bool) -> Result<PruneReport, AssetError> {
        self.lock()?
            .catalog()?
            .prune(&self.blobs, dry_run, |digest| self.package(digest))
    }
    pub fn pin(&self, owner: &str, digest: &str) -> Result<(), AssetError> {
        let lease = self.lock()?;
        lease.publish(digest, None)?;
        lease.catalog()?.pin(owner, digest)
    }
    pub fn unpin(&self, owner: &str) -> Result<(), AssetError> {
        self.lock()?.catalog()?.unpin(owner)
    }

    fn import_layout_unlocked(&self, layout: &Path) -> Result<PackageDescriptor, AssetError> {
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
            self.store_bytes_unlocked(descriptor, &bytes)?;
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
        digest: &str,
    ) -> Result<(), AssetError> {
        let lease = self.lock()?;
        lease.publish(digest, Some(reference)).map(|_| ())
    }

    pub fn resolve_reference(&self, reference: &AssetReference) -> Result<String, AssetError> {
        Catalog::open(&self.catalog_path())?.resolve(reference)
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

impl CacheLease {
    pub fn materialize(
        &self,
        digest: &str,
        destination: &Path,
    ) -> Result<crate::ValidatedAsset, AssetError> {
        crate::materialize::materialize_unlocked(&self.cache, digest, destination)
    }

    pub fn import_layout(
        &self,
        layout: &Path,
        reference: Option<&AssetReference>,
    ) -> Result<PackageDescriptor, AssetError> {
        if let Some(reference) = reference {
            reference.require_tag()?;
        }
        let package = self.cache.import_layout_unlocked(layout)?;
        self.publish(&package.manifest_digest, reference)
    }

    fn catalog(&self) -> Result<Catalog, AssetError> {
        Catalog::open(&self.cache.catalog_path())
    }
    pub(crate) fn publish(
        &self,
        digest: &str,
        reference: Option<&AssetReference>,
    ) -> Result<PackageDescriptor, AssetError> {
        let package = self.cache.package(digest)?;
        self.catalog()?.admit(&package, reference)?;
        Ok(package)
    }
    pub fn store_bytes(&self, descriptor: &OciDescriptor, bytes: &[u8]) -> Result<(), AssetError> {
        self.cache.store_bytes_unlocked(descriptor, bytes)
    }
    pub(crate) fn persist(
        &self,
        descriptor: &OciDescriptor,
        temporary: tempfile::NamedTempFile,
    ) -> Result<(), AssetError> {
        self.cache.persist(descriptor, temporary)
    }
}

fn ensure_owned_directory(path: &Path) -> Result<(), AssetError> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => return Ok(()),
        Ok(_) => {
            return Err(AssetError::invalid(format!(
                "store directory must not be a link or non-directory: {}",
                path.display()
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(AssetError::read(path, error)),
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|error| AssetError::read(path, error))
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
