//! Endpoint resolution + tonic Channel construction.
//!
//! Resolution order matches the user-facing answer:
//!   1. `--endpoint` flag (explicit, no auto-discovery).
//!   2. `OSDL_ENDPOINT` env var (same as flag).
//!   3. `--instance NAME` / `OSDL_INSTANCE` (look up that name in the lockfile dir).
//!   4. Auto-discovery: scan the lockfile dir; if exactly one live instance,
//!      connect to it; if zero, error with a hint; if multiple, list them
//!      and require disambiguation.

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use osdl_proto::v1::osdl_client::OsdlClient;
use osdl_server::{lockfile, paths::Paths};
use tonic::codegen::InterceptedService;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Request, Status};

/// All client commands receive this concrete type. We always wrap the
/// channel in an `InterceptedService` so that the optional bearer-token
/// path is uniform — the no-token interceptor is a no-op pass-through.
pub type Client = OsdlClient<InterceptedService<Channel, AuthInterceptor>>;

#[derive(Debug, Clone)]
pub enum Resolved {
    /// Connect over the given URI (typically `http://host:port`).
    Tcp(Uri),
    /// Connect over the given Unix domain socket path.
    Uds(PathBuf),
}

#[derive(Debug, Clone, Default)]
pub struct ClientOpts {
    pub endpoint: Option<String>,
    pub instance: Option<String>,
    /// Bearer token attached to TCP requests when the server has auth
    /// enabled. UDS doesn't need it but the server happily ignores
    /// extra metadata, so we always attach when set.
    pub auth_token: Option<String>,
}

#[derive(Clone)]
pub struct AuthInterceptor {
    bearer: Option<MetadataValue<tonic::metadata::Ascii>>,
}

impl AuthInterceptor {
    fn new(token: Option<&str>) -> anyhow::Result<Self> {
        let bearer = match token {
            None => None,
            Some(t) => Some(
                MetadataValue::try_from(format!("Bearer {t}"))
                    .context("invalid auth token (must be ASCII)")?,
            ),
        };
        Ok(Self { bearer })
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        if let Some(b) = &self.bearer {
            req.metadata_mut().insert("authorization", b.clone());
        }
        Ok(req)
    }
}

pub fn resolve(opts: &ClientOpts, paths: &Paths) -> anyhow::Result<Resolved> {
    if let Some(ep) = opts.endpoint.clone() {
        return parse_explicit(&ep);
    }

    if let Some(name) = &opts.instance {
        let rec = lockfile::find(&paths.lock_dir(), name).ok_or_else(|| {
            anyhow!(
                "no running osdl server with instance '{name}' — start one with 'osdl serve --instance {name}'"
            )
        })?;
        return record_to_resolved(&rec);
    }

    // Auto-discovery.
    let entries = lockfile::list(&paths.lock_dir());
    match entries.len() {
        0 => Err(anyhow!(
            "no running osdl server — start one with 'osdl serve' (looked in {})",
            paths.lock_dir().display()
        )),
        1 => record_to_resolved(&entries[0]),
        _ => {
            let names: Vec<&str> = entries.iter().map(|e| e.instance.as_str()).collect();
            Err(anyhow!(
                "multiple osdl servers running ({}) — pick one with --instance NAME or OSDL_INSTANCE=NAME",
                names.join(", ")
            ))
        }
    }
}

fn parse_explicit(ep: &str) -> anyhow::Result<Resolved> {
    if let Some(rest) = ep.strip_prefix("unix:") {
        return Ok(Resolved::Uds(PathBuf::from(rest)));
    }
    let uri: Uri = ep.parse().with_context(|| format!("invalid endpoint: {ep}"))?;
    Ok(Resolved::Tcp(uri))
}

fn record_to_resolved(rec: &lockfile::InstanceRecord) -> anyhow::Result<Resolved> {
    if let Some(p) = &rec.socket_path {
        return Ok(Resolved::Uds(p.clone()));
    }
    if let Some(addr) = &rec.listen_addr {
        let uri: Uri = format!("http://{addr}")
            .parse()
            .with_context(|| format!("listen_addr {addr} is not a valid URI"))?;
        return Ok(Resolved::Tcp(uri));
    }
    Err(anyhow!(
        "instance '{}' lockfile has no listener info — server may be misconfigured",
        rec.instance
    ))
}

/// Build a tonic Channel for the resolved endpoint. UDS uses
/// `Endpoint::try_from("http://[::]")` + a custom connector — tonic doesn't
/// have first-class UDS-client support, so we rely on tower service
/// composition.
pub async fn connect(resolved: &Resolved, opts: &ClientOpts) -> anyhow::Result<Client> {
    let interceptor = AuthInterceptor::new(opts.auth_token.as_deref())?;
    let channel = match resolved {
        Resolved::Tcp(uri) => {
            let endpoint = Endpoint::from_shared(uri.to_string())
                .with_context(|| format!("invalid TCP endpoint: {uri}"))?;
            endpoint
                .connect()
                .await
                .with_context(|| format!("connect to {uri}"))?
        }
        Resolved::Uds(path) => {
            let path_for_err = path.clone();
            let path_for_connector = path.clone();
            // The `http://[::]` URI is a placeholder — tonic requires *some*
            // authority, but the connector below ignores it.
            let endpoint = Endpoint::try_from("http://[::]")?;
            endpoint
                .connect_with_connector(tower::service_fn(move |_| {
                    let path = path_for_connector.clone();
                    async move {
                        let stream = tokio::net::UnixStream::connect(&path).await?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                    }
                }))
                .await
                .with_context(|| format!("connect to UDS {}", path_for_err.display()))?
        }
    };
    Ok(OsdlClient::with_interceptor(channel, interceptor))
}
