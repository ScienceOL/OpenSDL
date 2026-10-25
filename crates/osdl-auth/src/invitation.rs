use anyhow::{bail, ensure, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Dev,
    Beta,
    Rc,
    Stable,
}
impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Beta => "beta",
            Self::Rc => "rc",
            Self::Stable => "stable",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Read,
    Operate,
}
// Intentionally not Debug: the capability is a secret.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub lab_id: String,
    pub instance: String,
    pub channel: Channel,
    pub endpoint: String,
    pub ca_pem: String,
    pub invitation_id: String,
    pub capability: String,
    pub expires_at: u64,
    pub access: Access,
    pub grant_expires_at: Option<u64>,
}
impl Invitation {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 8192, "invitation_too_large");
        let encoded = text
            .strip_prefix("opensdl-pair-v1.")
            .ok_or_else(|| anyhow::anyhow!("invalid_invitation"))?;
        let bytes = B64
            .decode(encoded)
            .map_err(|_| anyhow::anyhow!("invalid_invitation"))?;
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid_invitation"))?;
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        let result = format!("opensdl-pair-v1.{}", B64.encode(serde_json::to_vec(self)?));
        ensure!(result.len() <= 8192, "invitation_too_large");
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        for id in [&self.lab_id, &self.invitation_id] {
            ensure!(
                uuid::Uuid::parse_str(id)?.to_string() == *id,
                "invalid_identity"
            );
        }
        ensure!(
            crate::valid_instance_name(&self.instance),
            "invalid_instance"
        );
        ensure!(
            normalize_endpoint(&self.endpoint)? == self.endpoint,
            "invalid_endpoint"
        );
        ensure!(
            B64.decode(&self.capability).is_ok_and(|v| v.len() == 32),
            "invalid_capability"
        );
        ensure!(
            self.expires_at > crate::now() && self.expires_at <= crate::now() + 600,
            "invitation_expired"
        );
        ensure!(
            self.grant_expires_at.is_none_or(|v| v > crate::now()),
            "grant_expired"
        );
        let (_, pem) = x509_parser::pem::parse_x509_pem(self.ca_pem.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid_ca"))?;
        let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents)
            .map_err(|_| anyhow::anyhow!("invalid_ca"))?;
        ensure!(cert.is_ca() && cert.validity().is_valid(), "invalid_ca");
        Ok(())
    }
    pub fn server_name(&self) -> String {
        format!("lab-{}.opensdl.invalid", self.lab_id)
    }
}
pub fn normalize_endpoint(raw: &str) -> Result<String> {
    let raw = raw.strip_prefix("https://").unwrap_or(raw);
    ensure!(
        !raw.contains('/') && !raw.contains('@') && !raw.contains('?') && !raw.contains('#'),
        "invalid_endpoint"
    );
    let (host, port) = match raw.split_once(':') {
        Some((h, p)) => (h, p.parse::<u16>()?),
        None => (raw, 50051),
    };
    let ip: std::net::Ipv4Addr = host.parse()?;
    let oct = ip.octets();
    if oct[0] != 100 || !(64..128).contains(&oct[1]) || port == 0 {
        bail!("invalid_endpoint");
    }
    ensure!(host == ip.to_string(), "invalid_endpoint");
    Ok(format!("https://{ip}:{port}"))
}
