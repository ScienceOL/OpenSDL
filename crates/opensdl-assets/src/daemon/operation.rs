use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    materialize, pack_directory, AssetCache, AssetError, AssetReference, RegistryClient,
    RegistryCredentials,
};

/// One contract shared by the local and daemon execution paths.
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Operation {
    Ping,
    Shutdown,
    List,
    Inspect {
        reference: String,
    },
    Tag {
        source: String,
        target: String,
    },
    Remove {
        reference: String,
    },
    Prune {
        dry_run: bool,
    },
    Import {
        layout: PathBuf,
        tag: Option<String>,
    },
    Build {
        directory: PathBuf,
        output: Option<PathBuf>,
        tag: Option<String>,
    },
    Pull {
        reference: String,
        output: Option<PathBuf>,
        offline: bool,
        credentials: RegistryCredentials,
    },
    Push {
        source: String,
        reference: String,
        credentials: RegistryCredentials,
    },
}

impl Operation {
    pub(crate) async fn execute(self, cache: &AssetCache) -> Result<Value, AssetError> {
        match self {
            Self::Ping => encode(serde_json::json!({"pid": std::process::id()})),
            Self::Shutdown => Err(AssetError::invalid("shutdown requires a daemon")),
            Self::List => encode(blocking(cache, |cache| cache.list()).await?),
            Self::Inspect { reference } => {
                encode(blocking(cache, move |cache| cache.inspect(&reference)).await?)
            }
            Self::Tag { source, target } => {
                let target = AssetReference::parse(&target)?;
                blocking(cache, move |cache| cache.tag(&source, &target)).await?;
                Ok(Value::Null)
            }
            Self::Remove { reference } => {
                blocking(cache, move |cache| cache.remove(&reference)).await?;
                Ok(Value::Null)
            }
            Self::Prune { dry_run } => {
                encode(blocking(cache, move |cache| cache.prune(dry_run)).await?)
            }
            Self::Import { layout, tag } => {
                let tag = parse_tag(tag)?;
                encode(
                    blocking(cache, move |cache| {
                        cache.lock()?.import_layout(&layout, tag.as_ref())
                    })
                    .await?,
                )
            }
            Self::Build {
                directory,
                output,
                tag,
            } => {
                let tag = parse_tag(tag)?;
                encode(
                    blocking(cache, move |cache| {
                        let temporary;
                        let layout = match output {
                            Some(output) => output,
                            None => {
                                temporary = tempfile::tempdir_in(cache.root().join("staging"))
                                    .map_err(|e| AssetError::read(cache.root(), e))?;
                                temporary.path().join("layout")
                            }
                        };
                        pack_directory(&directory, &layout)?;
                        cache.lock()?.import_layout(&layout, tag.as_ref())
                    })
                    .await?,
                )
            }
            Self::Pull {
                reference,
                output,
                offline,
                credentials,
            } => {
                let reference = AssetReference::parse(&reference)?;
                let result = RegistryClient::for_reference(&reference)
                    .pull(&reference, cache, &credentials, offline)
                    .await?;
                if let Some(output) = output {
                    let digest = result.resolved_digest.clone();
                    blocking(cache, move |cache| materialize(&cache, &digest, &output)).await?;
                }
                encode(result)
            }
            Self::Push {
                source,
                reference,
                credentials,
            } => {
                let reference = AssetReference::parse(&reference)?;
                encode(
                    RegistryClient::for_reference(&reference)
                        .push_cached(cache, &source, &reference, &credentials)
                        .await?,
                )
            }
        }
    }
}

fn parse_tag(tag: Option<String>) -> Result<Option<AssetReference>, AssetError> {
    tag.map(|tag| {
        let reference = AssetReference::parse(&tag)?;
        reference.require_tag()?;
        Ok(reference)
    })
    .transpose()
}

fn encode(value: impl Serialize) -> Result<Value, AssetError> {
    serde_json::to_value(value)
        .map_err(|e| AssetError::invalid(format!("encode asset result: {e}")))
}

async fn blocking<T: Send + 'static>(
    cache: &AssetCache,
    operation: impl FnOnce(AssetCache) -> Result<T, AssetError> + Send + 'static,
) -> Result<T, AssetError> {
    let cache = cache.clone();
    tokio::task::spawn_blocking(move || operation(cache))
        .await
        .map_err(|e| AssetError::invalid(format!("asset worker failed: {e}")))?
}
