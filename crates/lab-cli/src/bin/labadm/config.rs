use std::{fmt, net::SocketAddr, path::Path, str::FromStr};

use anyhow::{ensure, Context};
use serde::{de::DeserializeOwned, Deserialize};
use tonic::transport::Endpoint;

use crate::tls::TlsFiles;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(try_from = "String")]
pub struct Id(String);

impl Id {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn worker_tls_name(&self, cluster: &Id) -> String {
        format!("{self}.{cluster}.worker.opensdl.internal")
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Id {
    type Error = anyhow::Error;

    fn try_from(value: String) -> anyhow::Result<Self> {
        let alphanumeric = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
        let bytes = value.as_bytes();
        ensure!(
            (1..=63).contains(&bytes.len())
                && alphanumeric(bytes[0])
                && alphanumeric(bytes[bytes.len() - 1])
                && bytes.iter().all(|&c| alphanumeric(c) || c == b'-'),
            "cluster and worker IDs must be lowercase DNS labels"
        );
        Ok(Self(value))
    }
}

impl FromStr for Id {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> anyhow::Result<Self> {
        value.to_owned().try_into()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    pub cluster: Id,
    pub listen: SocketAddr,
    pub incoming: TlsFiles,
    pub workers: TlsFiles,
    pub directory: DirectoryConfig,
    pub account_gateway: Option<AccountGatewayConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountGatewayConfig {
    pub listen: SocketAddr,
    pub instance: String,
    pub media_relay: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryConfig {
    pub endpoints: Vec<String>,
    pub tls: TlsFiles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub cluster: Id,
    pub worker: Id,
    pub listen: SocketAddr,
    pub incoming: TlsFiles,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorConfig {
    pub cluster: Id,
    pub endpoints: Vec<String>,
    pub tls: TlsFiles,
}

pub fn read<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_yaml::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

pub fn https_endpoint(address: String) -> anyhow::Result<Endpoint> {
    let endpoint = Endpoint::from_shared(address)?;
    let uri = endpoint.uri();
    ensure!(
        uri.scheme_str() == Some("https")
            && uri.authority().is_some_and(|a| !a.as_str().contains('@'))
            && uri.path() == "/"
            && uri.query().is_none(),
        "endpoints must be HTTPS origins"
    );
    Ok(endpoint)
}

pub fn worker_address(address: SocketAddr) -> anyhow::Result<SocketAddr> {
    ensure!(
        address.port() != 0 && !address.ip().is_unspecified() && !address.ip().is_multicast(),
        "worker address must be a routable IP and port"
    );
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_cannot_escape_the_directory_scope() {
        for name in ["", ".", "../lab", "lab/w", "-node", "node-", "UPPER", "a_b"] {
            assert!(name.parse::<Id>().is_err(), "{name}");
        }
        let worker = "node-1".parse::<Id>().unwrap();
        assert_eq!(
            worker.worker_tls_name(&"lab-a".parse().unwrap()),
            "node-1.lab-a.worker.opensdl.internal"
        );
    }

    #[test]
    fn accepts_only_https_origins() {
        for address in [
            "http://a:80",
            "https://a:443/x",
            "https://user@a",
            "https://a/?q=x",
        ] {
            assert!(https_endpoint(address.into()).is_err(), "{address}");
        }
        assert!(https_endpoint("https://[::1]:50051".into()).is_ok());
        assert!(https_endpoint("https://lab.example:50051".into()).is_ok());
    }
}
