use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::paths::{validate_asset_tree, validate_gltf_dependencies, ValidatedPaths};

pub const API_VERSION: &str = "assets.opensdl.org/v1alpha1";
pub const KIND: &str = "VirtualAsset";

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("I/O at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid asset.yaml: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
    #[error("invalid virtual asset: {0}")]
    Invalid(String),
    #[error("registry {operation}: {message}")]
    Registry {
        operation: &'static str,
        message: String,
    },
}

impl AssetError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub(crate) fn read(path: &Path, source: std::io::Error) -> Self {
        Self::Read {
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn registry(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self::Registry {
            operation,
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualAssetManifest {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: AssetMetadata,
    pub spec: AssetSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetMetadata {
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub license: AssetLicense,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetLicense {
    pub spdx: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetSpec {
    #[serde(rename = "type")]
    pub asset_type: AssetType,
    pub source: AssetSource,
    pub preview: AssetPreview,
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub compatibility: AssetCompatibility,
    #[serde(default)]
    pub bindings: AssetBindings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetType {
    Instrument,
    Robot,
    Sensor,
    Prop,
    Scene,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSource {
    pub format: SourceFormat,
    pub root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFormat {
    Gltf,
    OpenUsd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPreview {
    pub model: String,
    pub thumbnail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Visual,
    Scene,
    Simulation,
    OpensdlDevice,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetCompatibility {
    #[serde(default)]
    pub standards: Vec<String>,
    #[serde(default)]
    pub runtimes: Vec<RuntimeCompatibility>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCompatibility {
    pub name: String,
    pub versions: Vec<String>,
    pub status: CompatibilityStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompatibilityStatus {
    Expected,
    Tested,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetBindings {
    #[serde(default)]
    pub device_types: Vec<String>,
}

#[derive(Debug)]
pub struct ValidatedAsset {
    pub root: PathBuf,
    pub manifest: VirtualAssetManifest,
    pub paths: ValidatedPaths,
}

impl VirtualAssetManifest {
    pub fn load(root: &Path) -> Result<ValidatedAsset, AssetError> {
        let root = validate_asset_tree(root)?;
        let manifest_path = root.join("asset.yaml");
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|source| AssetError::read(&manifest_path, source))?;
        let manifest: Self = serde_yaml::from_str(&manifest_text)?;
        manifest.validate_declaration()?;
        let paths = ValidatedPaths::resolve(&root, &manifest)?;
        validate_gltf_dependencies(&root, &manifest, &paths)?;

        Ok(ValidatedAsset {
            root,
            manifest,
            paths,
        })
    }

    /// Validate the portable declaration without consulting the asset tree.
    ///
    /// Registry clients use this after verifying an OCI config blob so a
    /// remotely supplied package is held to the same schema and semantic
    /// constraints as a directory passed to `lab validate` or `lab pack`.
    pub fn validate_declaration(&self) -> Result<(), AssetError> {
        self.validate_metadata()?;
        self.validate_capabilities()
    }

    fn validate_metadata(&self) -> Result<(), AssetError> {
        if self.api_version != API_VERSION {
            return Err(AssetError::invalid(format!(
                "unsupported apiVersion {:?}; expected {API_VERSION}",
                self.api_version
            )));
        }
        if self.kind != KIND {
            return Err(AssetError::invalid(format!(
                "unsupported kind {:?}; expected {KIND}",
                self.kind
            )));
        }
        if !is_dns_label(&self.metadata.name) {
            return Err(AssetError::invalid(
                "metadata.name must be a lower-case DNS label",
            ));
        }
        if self.metadata.display_name.trim().is_empty() {
            return Err(AssetError::invalid(
                "metadata.displayName must not be empty",
            ));
        }
        if self.metadata.description.trim().is_empty() {
            return Err(AssetError::invalid(
                "metadata.description must not be empty",
            ));
        }
        if self.metadata.version.starts_with('v')
            || semver::Version::parse(&self.metadata.version).is_err()
        {
            return Err(AssetError::invalid(
                "metadata.version must be a semantic version without a leading v",
            ));
        }
        spdx::Expression::parse(&self.metadata.license.spdx).map_err(|_| {
            AssetError::invalid("metadata.license.spdx must be a valid SPDX expression")
        })?;
        reject_empty_or_duplicate("metadata.tags", &self.metadata.tags)?;
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), AssetError> {
        let capabilities: HashSet<_> = self.spec.capabilities.iter().copied().collect();
        if capabilities.len() != self.spec.capabilities.len() {
            return Err(AssetError::invalid(
                "spec.capabilities must not contain duplicates",
            ));
        }
        if self.spec.source.format == SourceFormat::Gltf
            && capabilities.contains(&Capability::Simulation)
        {
            return Err(AssetError::invalid(
                "gltf cannot claim simulation capability",
            ));
        }
        if self.spec.source.format == SourceFormat::Gltf
            && capabilities.contains(&Capability::OpensdlDevice)
        {
            return Err(AssetError::invalid(
                "gltf cannot claim opensdl-device capability",
            ));
        }
        if capabilities.contains(&Capability::OpensdlDevice)
            && self.spec.bindings.device_types.is_empty()
        {
            return Err(AssetError::invalid(
                "opensdl-device requires at least one spec.bindings.deviceTypes entry",
            ));
        }
        reject_empty_or_duplicate(
            "spec.bindings.deviceTypes",
            &self.spec.bindings.device_types,
        )?;
        reject_empty_or_duplicate(
            "spec.compatibility.standards",
            &self.spec.compatibility.standards,
        )?;
        for runtime in &self.spec.compatibility.runtimes {
            if runtime.name.trim().is_empty() || runtime.versions.is_empty() {
                return Err(AssetError::invalid(
                    "compatibility runtime requires a name and at least one version",
                ));
            }
            reject_empty_or_duplicate("runtime.versions", &runtime.versions)?;
        }
        Ok(())
    }
}

fn is_dns_label(value: &str) -> bool {
    if value.is_empty() || value.len() > 63 {
        return false;
    }
    let bytes = value.as_bytes();
    let is_alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    is_alphanumeric(bytes[0])
        && is_alphanumeric(bytes[bytes.len() - 1])
        && bytes
            .iter()
            .all(|byte| is_alphanumeric(*byte) || *byte == b'-')
}

fn reject_empty_or_duplicate(field: &str, values: &[String]) -> Result<(), AssetError> {
    let mut seen = HashSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(AssetError::invalid(format!(
                "{field} must not contain empty values"
            )));
        }
        if !seen.insert(value) {
            return Err(AssetError::invalid(format!(
                "{field} must not contain duplicates"
            )));
        }
    }
    Ok(())
}
