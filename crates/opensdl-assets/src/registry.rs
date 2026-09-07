use std::path::Path;

use http::HeaderValue;
use oci_client::client::{ClientConfig, ClientProtocol};
use oci_client::secrets::RegistryAuth;
use oci_client::Client;

use crate::cache::AssetCache;
use crate::layout::{validate_manifest_shape, verify_blob};
use crate::manifest::AssetError;
use crate::media_types::{ARTIFACT_TYPE, MANIFEST_MEDIA_TYPE};
use crate::package::{digest_bytes, ImageManifest, OciDescriptor, PackageDescriptor};
use crate::reference::AssetReference;

#[derive(Clone, PartialEq, Eq)]
pub enum RegistryCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Bearer(String),
}

impl RegistryCredentials {
    fn as_oci(&self) -> RegistryAuth {
        match self {
            Self::Anonymous => RegistryAuth::Anonymous,
            Self::Basic { username, password } => {
                RegistryAuth::Basic(username.clone(), password.clone())
            }
            Self::Bearer(token) => RegistryAuth::Bearer(token.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushResult {
    pub reference: String,
    pub manifest_digest: String,
    pub uploaded_blobs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullResult {
    pub reference: String,
    pub resolved_digest: String,
    pub downloaded_blobs: usize,
    pub reused_blobs: usize,
    pub package: PackageDescriptor,
}

pub struct RegistryClient {
    insecure_loopback_registry: Option<String>,
}

impl RegistryClient {
    pub fn for_reference(reference: &AssetReference) -> Self {
        Self {
            insecure_loopback_registry: is_loopback_registry(reference.registry())
                .then(|| reference.registry().to_owned()),
        }
    }

    fn client(&self) -> Client {
        let mut config = ClientConfig::default();
        if let Some(registry) = &self.insecure_loopback_registry {
            config.protocol = ClientProtocol::HttpsExcept(vec![registry.clone()]);
        }
        Client::new(config)
    }

    pub async fn push_layout(
        &self,
        layout: &Path,
        reference: &AssetReference,
        credentials: &RegistryCredentials,
    ) -> Result<PushResult, AssetError> {
        if reference.tag().is_none() || reference.digest().is_some() {
            return Err(AssetError::invalid(
                "push requires a tag reference, not a digest reference",
            ));
        }
        let package = crate::inspect_layout(layout)?;
        let auth = credentials.as_oci();
        let client = self.client();
        client
            .store_auth_if_needed(reference.registry(), &auth)
            .await;

        let mut uploaded_blobs = 0_usize;
        for descriptor in std::iter::once(&package.config).chain(package.layers.iter()) {
            let bytes = verify_blob(layout, descriptor)?;
            client
                .push_blob(reference.as_oci(), bytes, &descriptor.digest)
                .await
                .map_err(|error| AssetError::registry("push blob", error))?;
            uploaded_blobs += 1;
        }

        let manifest_descriptor = OciDescriptor {
            media_type: MANIFEST_MEDIA_TYPE.to_owned(),
            digest: package.manifest_digest.clone(),
            size: package.manifest_size,
            artifact_type: Some(package.artifact_type.clone()),
            annotations: Default::default(),
        };
        let manifest_bytes = verify_blob(layout, &manifest_descriptor)?;
        client
            .push_manifest_raw(
                reference.as_oci(),
                manifest_bytes,
                HeaderValue::from_static(MANIFEST_MEDIA_TYPE),
            )
            .await
            .map_err(|error| AssetError::registry("push manifest", error))?;

        Ok(PushResult {
            reference: reference.to_string(),
            manifest_digest: package.manifest_digest,
            uploaded_blobs,
        })
    }

    pub async fn pull(
        &self,
        reference: &AssetReference,
        cache: &AssetCache,
        credentials: &RegistryCredentials,
        offline: bool,
    ) -> Result<PullResult, AssetError> {
        if offline {
            let resolved_digest = cache.resolve_reference(reference)?;
            let package = cache.package(&resolved_digest)?;
            return Ok(PullResult {
                reference: reference.to_string(),
                resolved_digest,
                reused_blobs: package.layers.len() + 2,
                downloaded_blobs: 0,
                package,
            });
        }

        let auth = credentials.as_oci();
        let client = self.client();
        let (manifest_bytes, resolved_digest) =
            self.pull_manifest(&client, reference, &auth).await?;
        verify_resolved_manifest(reference, &manifest_bytes, &resolved_digest)?;
        let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|error| AssetError::invalid(format!("parse pulled OCI manifest: {error}")))?;
        validate_manifest_shape(&manifest)?;

        let manifest_descriptor = OciDescriptor {
            media_type: MANIFEST_MEDIA_TYPE.to_owned(),
            digest: resolved_digest.clone(),
            size: manifest_bytes.len() as u64,
            artifact_type: Some(ARTIFACT_TYPE.to_owned()),
            annotations: Default::default(),
        };
        let mut downloaded_blobs = 0_usize;
        let mut reused_blobs = 0_usize;
        if cache.contains(&manifest_descriptor)? {
            reused_blobs += 1;
        } else {
            cache.store_bytes(&manifest_descriptor, &manifest_bytes)?;
            downloaded_blobs += 1;
        }

        for descriptor in std::iter::once(&manifest.config).chain(manifest.layers.iter()) {
            if cache.contains(descriptor)? {
                reused_blobs += 1;
                continue;
            }
            self.pull_blob(&client, reference, descriptor, cache)
                .await?;
            downloaded_blobs += 1;
        }

        cache.record_reference(reference, &resolved_digest)?;
        let package = cache.package(&resolved_digest)?;
        Ok(PullResult {
            reference: reference.to_string(),
            resolved_digest,
            downloaded_blobs,
            reused_blobs,
            package,
        })
    }

    async fn pull_manifest(
        &self,
        client: &Client,
        reference: &AssetReference,
        auth: &RegistryAuth,
    ) -> Result<(Vec<u8>, String), AssetError> {
        client
            .pull_manifest_raw(reference.as_oci(), auth, &[MANIFEST_MEDIA_TYPE])
            .await
            .map(|(bytes, digest)| (bytes.to_vec(), digest))
            .map_err(|error| AssetError::registry("pull manifest", error))
    }

    async fn pull_blob(
        &self,
        client: &Client,
        reference: &AssetReference,
        descriptor: &OciDescriptor,
        cache: &AssetCache,
    ) -> Result<(), AssetError> {
        let temporary = cache.temporary_blob()?;
        let std_file = temporary
            .reopen()
            .map_err(|source| AssetError::read(temporary.path(), source))?;
        let mut file = tokio::fs::File::from_std(std_file);
        client
            .pull_blob(reference.as_oci(), descriptor.digest.as_str(), &mut file)
            .await
            .map_err(|error| AssetError::registry("pull blob", error))?;
        file.sync_all()
            .await
            .map_err(|source| AssetError::read(temporary.path(), source))?;
        drop(file);
        cache.persist(descriptor, temporary)
    }
}

fn verify_resolved_manifest(
    reference: &AssetReference,
    bytes: &[u8],
    resolved_digest: &str,
) -> Result<(), AssetError> {
    let actual = digest_bytes(bytes);
    if actual != resolved_digest {
        return Err(AssetError::invalid(format!(
            "registry manifest digest mismatch: expected {resolved_digest}, got {actual}"
        )));
    }
    if reference
        .digest()
        .is_some_and(|expected| expected != resolved_digest)
    {
        return Err(AssetError::invalid(format!(
            "pinned manifest digest mismatch: expected {}, got {resolved_digest}",
            reference.digest().unwrap_or_default()
        )));
    }
    Ok(())
}

fn is_loopback_registry(registry: &str) -> bool {
    let host = registry
        .strip_prefix('[')
        .and_then(|value| value.split_once(']'))
        .map_or_else(
            || registry.split(':').next().unwrap_or(registry),
            |(host, _)| host,
        );
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}
