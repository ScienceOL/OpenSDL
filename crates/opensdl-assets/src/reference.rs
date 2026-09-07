use std::fmt;

use oci_client::Reference;

use crate::manifest::AssetError;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetReference {
    inner: Reference,
}

impl AssetReference {
    pub fn parse(value: &str) -> Result<Self, AssetError> {
        let value = value.strip_prefix("oci://").unwrap_or(value);
        if !has_explicit_registry(value) {
            return Err(AssetError::invalid(
                "asset reference must include an explicit registry host",
            ));
        }
        if !has_explicit_tag_or_digest(value) {
            return Err(AssetError::invalid(
                "asset reference requires an explicit tag or digest",
            ));
        }
        let inner: Reference = value
            .parse()
            .map_err(|error| AssetError::invalid(format!("invalid OCI reference: {error}")))?;
        if let Some(digest) = inner.digest() {
            validate_digest(digest)?;
        }
        Ok(Self { inner })
    }

    pub fn registry(&self) -> &str {
        self.inner.registry()
    }

    pub fn repository(&self) -> &str {
        self.inner.repository()
    }

    pub fn tag(&self) -> Option<&str> {
        self.inner.tag()
    }

    pub fn digest(&self) -> Option<&str> {
        self.inner.digest()
    }

    pub(crate) fn as_oci(&self) -> &Reference {
        &self.inner
    }
}

impl fmt::Display for AssetReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "oci://{}", self.inner)
    }
}

fn has_explicit_registry(value: &str) -> bool {
    let Some((registry, repository)) = value.split_once('/') else {
        return false;
    };
    !repository.is_empty()
        && (registry == "localhost" || registry.contains('.') || registry.contains(':'))
}

fn has_explicit_tag_or_digest(value: &str) -> bool {
    if value.contains('@') {
        return true;
    }
    value
        .rsplit_once('/')
        .is_some_and(|(_, final_component)| final_component.contains(':'))
}

pub(crate) fn validate_digest(digest: &str) -> Result<&str, AssetError> {
    digest
        .strip_prefix("sha256:")
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| AssetError::invalid(format!("invalid sha256 digest: {digest}")))
}
