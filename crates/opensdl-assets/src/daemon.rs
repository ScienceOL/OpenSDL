//! Authenticated, bounded loopback RPC for the local asset store.
mod operation;
pub use operation::Operation;

use crate::{AssetCache, AssetError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io::Write, net::SocketAddr, path::Path, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity {
    pid: u32,
    endpoint: SocketAddr,
    token: String,
}

#[derive(Debug, Serialize)]
pub struct DaemonRecord {
    pub pid: u32,
    pub endpoint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    operation: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "camelCase")]
enum Response {
    Success(Value),
    Error(String),
}

pub struct DaemonClient {
    identity: Identity,
}

/// CLI consumers share the same operation and presentation whether a daemon is running or not.
pub async fn execute<T: DeserializeOwned>(
    cache: &AssetCache,
    operation: Operation,
) -> Result<T, AssetError> {
    let value = match DaemonClient::discover(cache).await? {
        Some(client) => client.call(operation).await?,
        None => operation.execute(cache).await?,
    };
    serde_json::from_value(value)
        .map_err(|e| AssetError::invalid(format!("decode asset result: {e}")))
}

impl DaemonClient {
    pub async fn discover(cache: &AssetCache) -> Result<Option<Self>, AssetError> {
        let path = cache.root().join("daemon.json");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AssetError::read(&path, e)),
        };
        let identity: Identity = serde_json::from_slice(&bytes).map_err(|e| {
            AssetError::invalid(format!("invalid daemon record; restart the daemon: {e}"))
        })?;
        if !identity.endpoint.ip().is_loopback() {
            return Err(AssetError::invalid("daemon endpoint must be loopback"));
        }
        let stream = match tokio::time::timeout(
            PROBE_TIMEOUT,
            TcpStream::connect(identity.endpoint),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => return Ok(None),
            Ok(Err(e)) => return Err(AssetError::invalid(format!("connect daemon: {e}"))),
            Err(_) => return Err(AssetError::invalid("daemon connection timed out")),
        };
        let client = Self { identity };
        tokio::time::timeout(PROBE_TIMEOUT, client.exchange(stream, Operation::Ping))
            .await
            .map_err(|_| {
                AssetError::invalid("daemon health check timed out; inspect daemon.log")
            })??;
        Ok(Some(client))
    }

    pub fn record(&self) -> DaemonRecord {
        DaemonRecord {
            pid: self.identity.pid,
            endpoint: format!("tcp://{}", self.identity.endpoint),
        }
    }

    pub async fn call(&self, operation: Operation) -> Result<Value, AssetError> {
        let stream =
            tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(self.identity.endpoint))
                .await
                .map_err(|_| AssetError::invalid("daemon connection timed out"))?
                .map_err(|e| AssetError::invalid(format!("connect daemon: {e}")))?;
        tokio::time::timeout(OPERATION_TIMEOUT, self.exchange(stream, operation))
            .await
            .map_err(|_| {
                AssetError::invalid("daemon operation timed out; inspect state before retrying")
            })?
    }

    async fn exchange(
        &self,
        mut stream: TcpStream,
        operation: Operation,
    ) -> Result<Value, AssetError> {
        write_frame(
            &mut stream,
            &Request {
                token: self.identity.token.clone(),
                operation,
            },
        )
        .await?;
        let response: Response = read_frame(&mut stream).await?;
        match response {
            Response::Success(value) => Ok(value),
            Response::Error(error) => Err(AssetError::invalid(error)),
        }
    }
}

pub async fn serve(cache: AssetCache) -> Result<(), AssetError> {
    let lock_path = cache.root().join("daemon.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| AssetError::read(&lock_path, e))?;
    lock.try_lock()
        .map_err(|e| AssetError::invalid(format!("daemon is already running: {e}")))?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| AssetError::invalid(format!("bind daemon: {e}")))?;
    let identity = Identity {
        pid: std::process::id(),
        endpoint: listener
            .local_addr()
            .map_err(|e| AssetError::invalid(e.to_string()))?,
        token: uuid::Uuid::new_v4().simple().to_string(),
    };
    let record_path = cache.root().join("daemon.json");
    write_record(&record_path, &identity)?;
    let mut tasks = JoinSet::new();
    let mut shutdown = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|e| AssetError::invalid(format!("accept daemon client: {e}")))?;
                let cache = cache.clone();
                let token = identity.token.clone();
                tasks.spawn(async move { handle(stream, cache, &token).await });
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                match completed {
                    Some(Ok(Ok(Some(stream)))) => break stream,
                    Some(Ok(Err(error))) => eprintln!("daemon client: {error}"),
                    Some(Err(error)) => eprintln!("daemon task: {error}"),
                    _ => {}
                }
            }
        }
    };
    drop(listener);
    // Complete accepted operations before acknowledging shutdown; no interrupted admission or upload.
    while let Some(result) = tasks.join_next().await {
        if let Ok(Ok(Some(mut stream))) = result {
            if let Err(error) =
                write_frame(&mut stream, &Response::Error("daemon is stopping".into())).await
            {
                eprintln!("daemon shutdown client: {error}");
            }
        }
    }
    fs::remove_file(&record_path).map_err(|e| AssetError::read(&record_path, e))?;
    drop(lock);
    write_frame(&mut shutdown, &Response::Success(Value::Null)).await
}

async fn handle(
    mut stream: TcpStream,
    cache: AssetCache,
    token: &str,
) -> Result<Option<TcpStream>, AssetError> {
    let request: Request = tokio::time::timeout(PROBE_TIMEOUT, read_frame(&mut stream))
        .await
        .map_err(|_| AssetError::invalid("daemon request timed out"))??;
    if request.token != token {
        write_frame(
            &mut stream,
            &Response::Error("daemon authentication failed".into()),
        )
        .await?;
        return Ok(None);
    }
    if matches!(request.operation, Operation::Shutdown) {
        return Ok(Some(stream));
    }
    let result = request.operation.execute(&cache).await;
    let response = match result {
        Ok(value) => Response::Success(value),
        Err(error) => Response::Error(error.to_string()),
    };
    write_frame(&mut stream, &response).await?;
    Ok(None)
}

async fn read_frame<T: DeserializeOwned>(stream: impl AsyncRead + Unpin) -> Result<T, AssetError> {
    let mut bytes = Vec::new();
    BufReader::new(stream.take((MAX_FRAME_BYTES + 1) as u64))
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(|e| AssetError::invalid(format!("read daemon frame: {e}")))?;
    if bytes.len() > MAX_FRAME_BYTES || bytes.last() != Some(&b'\n') {
        return Err(AssetError::invalid(
            "daemon frame is too large or incomplete",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| AssetError::invalid(format!("decode daemon frame: {e}")))
}

async fn write_frame(stream: &mut TcpStream, value: &impl Serialize) -> Result<(), AssetError> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| AssetError::invalid(e.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(AssetError::invalid("daemon response is too large"));
    }
    tokio::time::timeout(PROBE_TIMEOUT, stream.write_all(&bytes))
        .await
        .map_err(|_| AssetError::invalid("daemon write timed out"))?
        .map_err(|e| AssetError::invalid(format!("write daemon frame: {e}")))
}

fn write_record(path: &Path, record: &Identity) -> Result<(), AssetError> {
    let parent = path.parent().expect("record has a parent");
    // NamedTempFile is private on Unix; persist atomically replaces only our daemon record.
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| AssetError::read(parent, e))?;
    let bytes = serde_json::to_vec(record).map_err(|e| AssetError::invalid(e.to_string()))?;
    file.write_all(&bytes)
        .map_err(|e| AssetError::read(path, e))?;
    file.persist(path)
        .map_err(|e| AssetError::read(path, e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_are_bounded_before_json_decoding() {
        let bytes = vec![b' '; MAX_FRAME_BYTES + 1024];
        let error = read_frame::<Request>(std::io::Cursor::new(bytes))
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("too large"));
    }

    #[tokio::test]
    async fn daemon_requires_its_private_token_and_removes_identity_before_stopping() {
        let temp = tempfile::tempdir().unwrap();
        let cache = AssetCache::open(temp.path()).unwrap();
        let server = tokio::spawn(serve(cache.clone()));
        let client = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(client) = DaemonClient::discover(&cache).await.unwrap() {
                    break client;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut stream = TcpStream::connect(client.identity.endpoint).await.unwrap();
        write_frame(
            &mut stream,
            &Request {
                token: "wrong".into(),
                operation: Operation::Shutdown,
            },
        )
        .await
        .unwrap();
        let response: Response = read_frame(&mut stream).await.unwrap();
        assert!(matches!(response, Response::Error(error) if error.contains("authentication")));
        assert!(client.call(Operation::Ping).await.is_ok());
        client.call(Operation::Shutdown).await.unwrap();
        assert!(!cache.root().join("daemon.json").exists());
        server.await.unwrap().unwrap();
    }
}
