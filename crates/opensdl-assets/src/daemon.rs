//! Local asset-store daemon.
//!
//! The daemon owns the content-addressed cache and its catalog.  Commands use
//! a loopback JSON-RPC connection when the daemon is running, while retaining
//! the direct library path for offline scripts and first-time setup.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::{
    materialize, pack_directory, AssetCache, AssetError, AssetReference, RegistryClient,
    RegistryCredentials,
};

const RECORD_FILE: &str = "daemon.json";
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonRecord {
    pub pid: u32,
    pub endpoint: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DaemonClient {
    record: DaemonRecord,
}

impl AssetCache {
    pub fn daemon_record_path(&self) -> PathBuf {
        self.root().join(RECORD_FILE)
    }
}

impl DaemonClient {
    pub async fn discover(cache: &AssetCache) -> Result<Option<Self>, AssetError> {
        let path = cache.daemon_record_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(AssetError::read(&path, error)),
        };
        let record: DaemonRecord = match serde_json::from_slice(&bytes) {
            Ok(record) => record,
            Err(_) => {
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
        };
        let client = Self { record };
        if client.call("ping", json!({})).await.is_ok() {
            Ok(Some(client))
        } else {
            let _ = fs::remove_file(path);
            Ok(None)
        }
    }

    pub async fn wait(cache: &AssetCache, timeout: Duration) -> Result<Self, AssetError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(client) = Self::discover(cache).await? {
                return Ok(client);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AssetError::invalid("asset daemon did not become ready"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn record(&self) -> &DaemonRecord {
        &self.record
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, AssetError> {
        let mut stream = connect(&self.record.endpoint).await?;
        let request = serde_json::to_vec(&Request {
            method: method.to_owned(),
            params,
        })
        .map_err(|error| AssetError::invalid(format!("serialize daemon request: {error}")))?;
        stream
            .write_all(&request)
            .await
            .map_err(|error| AssetError::invalid(format!("write daemon request: {error}")))?;
        stream
            .write_all(b"\n")
            .await
            .map_err(|error| AssetError::invalid(format!("write daemon request: {error}")))?;
        stream
            .flush()
            .await
            .map_err(|error| AssetError::invalid(format!("flush daemon request: {error}")))?;

        let mut line = Vec::new();
        let mut reader = BufReader::new(stream);
        let bytes = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|error| AssetError::invalid(format!("read daemon response: {error}")))?;
        if bytes == 0 || line.len() > MAX_REQUEST_BYTES {
            return Err(AssetError::invalid(
                "asset daemon returned an empty response",
            ));
        }
        let response: Response = serde_json::from_slice(&line)
            .map_err(|error| AssetError::invalid(format!("parse daemon response: {error}")))?;
        if response.ok {
            Ok(response.result.unwrap_or(Value::Null))
        } else {
            Err(AssetError::invalid(response.error.unwrap_or_else(|| {
                "asset daemon request failed".to_owned()
            })))
        }
    }

    pub async fn call_json<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, AssetError> {
        let value = self.call(method, params).await?;
        serde_json::from_value(value)
            .map_err(|error| AssetError::invalid(format!("decode daemon result: {error}")))
    }
}

pub async fn serve(cache: AssetCache) -> Result<(), AssetError> {
    let daemon_lock_path = cache.root().join("daemon.lock");
    let daemon_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&daemon_lock_path)
        .map_err(|error| AssetError::read(&daemon_lock_path, error))?;
    daemon_lock
        .lock()
        .map_err(|error| AssetError::read(&daemon_lock_path, error))?;
    let record_path = cache.daemon_record_path();
    if let Some(existing) = read_record(&record_path)? {
        if (DaemonClient {
            record: existing.clone(),
        })
        .call("ping", json!({}))
        .await
        .is_ok()
        {
            return Err(AssetError::invalid("asset daemon is already running"));
        }
        let _ = fs::remove_file(&record_path);
    }

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| AssetError::invalid(format!("bind asset daemon: {error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| AssetError::invalid(format!("read daemon address: {error}")))?;
    let record = DaemonRecord {
        pid: std::process::id(),
        endpoint: format!("tcp://{address}"),
    };
    write_record(&record_path, &record)?;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() { break; }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted
                    .map_err(|error| AssetError::invalid(format!("accept daemon client: {error}")))?;
                let cache = cache.clone();
                let shutdown = shutdown_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, cache, shutdown).await {
                        eprintln!("asset daemon client ended: {error}");
                    }
                });
            }
        }
    }
    let _ = fs::remove_file(record_path);
    drop(daemon_lock);
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    cache: AssetCache,
    shutdown: watch::Sender<bool>,
) -> Result<(), AssetError> {
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let bytes = reader
        .read_until(b'\n', &mut line)
        .await
        .map_err(|error| AssetError::invalid(format!("read daemon request: {error}")))?;
    if bytes == 0 {
        return Ok(());
    }
    if line.len() > MAX_REQUEST_BYTES {
        return write_response(
            reader.get_mut(),
            Err(AssetError::invalid("daemon request is too large")),
        )
        .await;
    }
    let request: Request = match serde_json::from_slice(&line) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                reader.get_mut(),
                Err(AssetError::invalid(format!(
                    "parse daemon request: {error}"
                ))),
            )
            .await;
        }
    };
    let is_shutdown = request.method == "shutdown";
    let result = dispatch(&cache, &request.method, request.params).await;
    write_response(reader.get_mut(), result).await?;
    if is_shutdown {
        let _ = shutdown.send(true);
    }
    Ok(())
}

async fn write_response(
    stream: &mut TcpStream,
    result: Result<Value, AssetError>,
) -> Result<(), AssetError> {
    let response = match result {
        Ok(result) => Response {
            ok: true,
            result: Some(result),
            error: None,
        },
        Err(error) => Response {
            ok: false,
            result: None,
            error: Some(error.to_string()),
        },
    };
    let bytes = serde_json::to_vec(&response)
        .map_err(|error| AssetError::invalid(format!("serialize daemon response: {error}")))?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|error| AssetError::invalid(format!("write daemon response: {error}")))?;
    stream
        .write_all(b"\n")
        .await
        .map_err(|error| AssetError::invalid(format!("write daemon response: {error}")))?;
    stream
        .flush()
        .await
        .map_err(|error| AssetError::invalid(format!("flush daemon response: {error}")))
}

async fn dispatch(cache: &AssetCache, method: &str, params: Value) -> Result<Value, AssetError> {
    match method {
        "ping" => Ok(json!({"pid": std::process::id()})),
        "shutdown" => Ok(json!({"stopping": true})),
        "list" => serde_json::to_value(blocking(cache, |cache| cache.list()).await?)
            .map_err(|error| AssetError::invalid(format!("encode asset list: {error}"))),
        "inspect" => {
            let selector = string_param(&params, "reference")?.to_owned();
            serde_json::to_value(blocking(cache, move |cache| cache.inspect(&selector)).await?)
                .map_err(|error| AssetError::invalid(format!("encode asset inspection: {error}")))
        }
        "tag" => {
            let source = string_param(&params, "source")?.to_owned();
            let target = AssetReference::parse(string_param(&params, "target")?)?;
            blocking(cache, move |cache| cache.tag(&source, &target)).await?;
            Ok(Value::Null)
        }
        "remove" => {
            let selector = string_param(&params, "reference")?.to_owned();
            blocking(cache, move |cache| cache.remove(&selector)).await?;
            Ok(Value::Null)
        }
        "prune" => {
            let dry_run = params
                .get("dryRun")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            serde_json::to_value(blocking(cache, move |cache| cache.prune(dry_run)).await?)
                .map_err(|error| AssetError::invalid(format!("encode prune report: {error}")))
        }
        "import" => {
            let layout = PathBuf::from(string_param(&params, "layout")?);
            let tag = params
                .get("tag")
                .and_then(Value::as_str)
                .map(AssetReference::parse)
                .transpose()?;
            let package = blocking(cache, move |cache| {
                cache.lock()?.import_layout(&layout, tag.as_ref())
            })
            .await?;
            serde_json::to_value(package)
                .map_err(|error| AssetError::invalid(format!("encode imported package: {error}")))
        }
        "materialize" => {
            let digest = string_param(&params, "digest")?;
            let destination = PathBuf::from(string_param(&params, "destination")?);
            let digest = digest.to_owned();
            blocking(cache, move |cache| {
                materialize(&cache, &digest, &destination)
            })
            .await?;
            Ok(Value::Null)
        }
        "build" => {
            let directory = PathBuf::from(string_param(&params, "directory")?);
            let output = params
                .get("output")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let tag = params
                .get("tag")
                .and_then(Value::as_str)
                .map(AssetReference::parse)
                .transpose()?;
            let package = blocking(cache, move |cache| {
                let temporary;
                let layout = if let Some(output) = output {
                    pack_directory(&directory, &output)?;
                    output
                } else {
                    temporary = tempfile::tempdir_in(cache.root().join("staging"))
                        .map_err(|error| AssetError::read(&cache.root().join("staging"), error))?;
                    let layout = temporary.path().join("layout");
                    pack_directory(&directory, &layout)?;
                    layout
                };
                cache.lock()?.import_layout(&layout, tag.as_ref())
            })
            .await?;
            serde_json::to_value(package)
                .map_err(|error| AssetError::invalid(format!("encode built package: {error}")))
        }
        "pull" => {
            let reference = AssetReference::parse(string_param(&params, "reference")?)?;
            let credentials: RegistryCredentials = params
                .get("credentials")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| {
                    AssetError::invalid(format!("decode registry credentials: {error}"))
                })?
                .unwrap_or(RegistryCredentials::Anonymous);
            let offline = params
                .get("offline")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let output = params
                .get("output")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let result = RegistryClient::for_reference(&reference)
                .pull(&reference, cache, &credentials, offline)
                .await?;
            if let Some(output) = output {
                let digest = result.resolved_digest.clone();
                blocking(cache, move |cache| materialize(&cache, &digest, &output)).await?;
            }
            serde_json::to_value(json!({
                "reference": result.reference,
                "resolvedDigest": result.resolved_digest,
                "downloadedBlobs": result.downloaded_blobs,
                "reusedBlobs": result.reused_blobs,
            }))
            .map_err(|error| AssetError::invalid(format!("encode pull result: {error}")))
        }
        "push" => {
            let source = string_param(&params, "source")?;
            let reference = AssetReference::parse(string_param(&params, "reference")?)?;
            let credentials: RegistryCredentials = params
                .get("credentials")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| {
                    AssetError::invalid(format!("decode registry credentials: {error}"))
                })?
                .unwrap_or(RegistryCredentials::Anonymous);
            let result = if Path::new(source).is_dir() {
                RegistryClient::for_reference(&reference)
                    .push_layout(Path::new(source), &reference, &credentials)
                    .await?
            } else {
                RegistryClient::for_reference(&reference)
                    .push_cached(cache, source, &reference, &credentials)
                    .await?
            };
            serde_json::to_value(result)
                .map_err(|error| AssetError::invalid(format!("encode push result: {error}")))
        }
        _ => Err(AssetError::invalid(format!(
            "unknown asset daemon method: {method}"
        ))),
    }
}

async fn blocking<T, F>(cache: &AssetCache, operation: F) -> Result<T, AssetError>
where
    T: Send + 'static,
    F: FnOnce(AssetCache) -> Result<T, AssetError> + Send + 'static,
{
    let cache = cache.clone();
    tokio::task::spawn_blocking(move || operation(cache))
        .await
        .map_err(|error| AssetError::invalid(format!("asset daemon worker failed: {error}")))?
}

fn string_param<'a>(params: &'a Value, name: &str) -> Result<&'a str, AssetError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AssetError::invalid(format!("daemon parameter {name} is required")))
}

async fn connect(endpoint: &str) -> Result<TcpStream, AssetError> {
    let address = endpoint
        .strip_prefix("tcp://")
        .ok_or_else(|| AssetError::invalid("unsupported asset daemon endpoint"))?;
    TcpStream::connect(address)
        .await
        .map_err(|error| AssetError::invalid(format!("connect asset daemon: {error}")))
}

fn read_record(path: &Path) -> Result<Option<DaemonRecord>, AssetError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| AssetError::invalid(format!("parse asset daemon record: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AssetError::read(path, error)),
    }
}

fn write_record(path: &Path, record: &DaemonRecord) -> Result<(), AssetError> {
    let bytes = serde_json::to_vec_pretty(record)
        .map_err(|error| AssetError::invalid(format!("serialize daemon record: {error}")))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|error| AssetError::read(&temporary, error))?;
    fs::rename(&temporary, path).map_err(|error| AssetError::read(path, error))
}
