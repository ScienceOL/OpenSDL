use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use serde::Deserialize;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity, ServerTlsConfig};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const RPC_TIMEOUT: Duration = Duration::from_secs(3);
const KEEPALIVE: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsFiles {
    pub certificate: PathBuf,
    pub private_key: PathBuf,
    pub ca: PathBuf,
}

pub struct Tls {
    identity: Identity,
    ca: Certificate,
}

impl TlsFiles {
    pub fn load(&self, config_path: &Path) -> anyhow::Result<Tls> {
        let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
        let read = |p: &Path| {
            let path = parent.join(p);
            std::fs::read(&path).with_context(|| format!("read TLS file {}", path.display()))
        };
        Ok(Tls {
            identity: Identity::from_pem(read(&self.certificate)?, read(&self.private_key)?),
            ca: Certificate::from_pem(read(&self.ca)?),
        })
    }
}

impl Tls {
    pub fn server(self) -> ServerTlsConfig {
        ServerTlsConfig::new()
            .identity(self.identity)
            .client_ca_root(self.ca)
    }

    pub fn endpoint(&self, endpoint: Endpoint, name: Option<String>) -> anyhow::Result<Endpoint> {
        let mut tls = ClientTlsConfig::new()
            .ca_certificate(self.ca.clone())
            .identity(self.identity.clone())
            .timeout(CONNECT_TIMEOUT);
        if let Some(name) = name {
            tls = tls.domain_name(name);
        }
        Ok(endpoint
            .tls_config(tls)?
            .connect_timeout(CONNECT_TIMEOUT)
            .http2_keep_alive_interval(KEEPALIVE)
            .keep_alive_timeout(CONNECT_TIMEOUT))
    }
}
