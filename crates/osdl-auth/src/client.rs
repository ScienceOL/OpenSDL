use crate::{Channel, Invitation, ProfileStore};
use anyhow::{ensure, Result};
use osdl_proto::v1 as pb;
use rcgen::PublicKeyData;
use serde::{Deserialize, Serialize};
use tonic::transport::{
    Certificate, Channel as TransportChannel, ClientTlsConfig, Endpoint, Identity,
};
#[cfg(all(test, windows))]
tokio::task_local! {
    static CONNECTION_ATTEMPTS: std::cell::Cell<usize>;
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    lab_id: String,
    instance: String,
    channel: Channel,
    endpoint: String,
    ca_pem: String,
    invitation_id: String,
    key_pem: String,
    csr: Vec<u8>,
    access: crate::Access,
    grant_expires_at: Option<u64>,
    credential: Credential,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
enum Credential {
    Pending,
    Paired {
        certificate: String,
        grant: String,
        certificate_expires_at: u64,
    },
}
/// Public local metadata only; operational authority is checked by connect and the service.
#[derive(Clone, Debug)]
pub struct ProfileMetadata {
    pub lab_id: String,
    pub instance: String,
    pub channel: Channel,
    pub endpoint: String,
    pub access: crate::Access,
    pub grant_expires_at: Option<u64>,
    pub state: ProfileState,
}
#[derive(Clone, Debug)]
pub enum ProfileState {
    Pending,
    Paired {
        grant_id: String,
        certificate_expires_at: u64,
    },
}
pub struct PendingProfile {
    record: Record,
}
impl PendingProfile {
    pub fn csr_der(&self) -> &[u8] {
        &self.record.csr
    }
}
#[derive(Clone, Debug)]
pub struct Laboratory {
    pub lab_id: String,
    pub instance: String,
    pub channel: String,
}
fn tls(lab: &str, ca: &str) -> ClientTlsConfig {
    crate::crypto_provider();
    ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(ca))
        .domain_name(format!("lab-{lab}.opensdl.invalid"))
}
async fn connect(endpoint: &str, config: ClientTlsConfig) -> Result<TransportChannel> {
    #[cfg(all(test, windows))]
    let _ = CONNECTION_ATTEMPTS.try_with(|counter| counter.set(counter.get() + 1));
    Ok(Endpoint::from_shared(endpoint.to_string())?
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15))
        .tls_config(config)?
        .connect()
        .await?)
}
async fn info(
    channel: TransportChannel,
    lab: &str,
    instance: &str,
    expected: Channel,
) -> Result<Laboratory> {
    let info = pb::authentication_client::AuthenticationClient::new(channel)
        .info(pb::AuthenticationInfoRequest {})
        .await?
        .into_inner();
    ensure!(
        info.mode == "public_key"
            && info.lab_id == lab
            && info.instance == instance
            && info.channel == expected.as_str(),
        "laboratory_identity_changed"
    );
    Ok(Laboratory {
        lab_id: info.lab_id,
        instance: info.instance,
        channel: info.channel,
    })
}
/// Authenticate the invitation's server-owned details before private confirmation, without mutation.
pub async fn inspect_invitation(inv: &Invitation) -> Result<pb::DescribeInvitationResponse> {
    inv.validate()?;
    let channel = connect(&inv.endpoint, tls(&inv.lab_id, &inv.ca_pem)).await?;
    info(channel.clone(), &inv.lab_id, &inv.instance, inv.channel).await?;
    let public = pb::pairing_client::PairingClient::new(channel)
        .describe_invitation(pb::DescribeInvitationRequest {
            invitation_id: inv.invitation_id.clone(),
            capability: inv.capability.clone(),
        })
        .await?
        .into_inner();
    ensure!(
        public.lab_id == inv.lab_id
            && public.instance == inv.instance
            && public.channel == inv.channel.as_str()
            && public.invitation_expires_at == inv.expires_at as i64
            && public.grant_expires_at == inv.grant_expires_at.map(|v| v as i64)
            && public.access
                == match inv.access {
                    crate::Access::Read => 1,
                    crate::Access::Operate => 2,
                },
        "invitation_binding_mismatch"
    );
    Ok(public)
}
impl ProfileStore {
    pub fn metadata(&self, name: &str) -> Result<ProfileMetadata> {
        let record: Record = serde_json::from_slice(&self.read_secret(name)?)
            .map_err(|_| anyhow::anyhow!("invalid_private_profile"))?;
        Ok(ProfileMetadata {
            lab_id: record.lab_id,
            instance: record.instance,
            channel: record.channel,
            endpoint: record.endpoint,
            access: record.access,
            grant_expires_at: record.grant_expires_at,
            state: match record.credential {
                Credential::Pending => ProfileState::Pending,
                Credential::Paired {
                    grant,
                    certificate_expires_at,
                    ..
                } => ProfileState::Paired {
                    grant_id: grant,
                    certificate_expires_at,
                },
            },
        })
    }
    pub fn prepare_pair(
        &self,
        name: &str,
        inv: &Invitation,
        replace: bool,
    ) -> Result<PendingProfile> {
        inv.validate()?;
        if self.contains(name)? && !replace {
            let record: Record = serde_json::from_slice(&self.read_secret(name)?)
                .map_err(|_| anyhow::anyhow!("invalid_private_profile"))?;
            ensure!(
                record.lab_id == inv.lab_id
                    && record.channel == inv.channel
                    && record.ca_pem == inv.ca_pem
                    && record.endpoint == inv.endpoint
                    && record.instance == inv.instance,
                "laboratory_identity_changed"
            );
            ensure!(record.invitation_id == inv.invitation_id, "profile_exists");
            return Ok(PendingProfile { record });
        }
        let key = rcgen::KeyPair::generate()?;
        let csr = rcgen::CertificateParams::default()
            .serialize_request(&key)?
            .der()
            .to_vec();
        let record = Record {
            lab_id: inv.lab_id.clone(),
            instance: inv.instance.clone(),
            channel: inv.channel,
            endpoint: inv.endpoint.clone(),
            ca_pem: inv.ca_pem.clone(),
            invitation_id: inv.invitation_id.clone(),
            key_pem: key.serialize_pem(),
            csr,
            access: inv.access,
            grant_expires_at: inv.grant_expires_at,
            credential: Credential::Pending,
        };
        self.write_secret(name, &serde_json::to_vec(&record)?, replace)?;
        Ok(PendingProfile { record })
    }
    /// The pending key is durable before this method sends the Pair request. Call only after confirmation.
    pub async fn pair(
        &self,
        name: &str,
        inv: &Invitation,
        label: &str,
        replace: bool,
    ) -> Result<pb::Grant> {
        ensure!(!label.is_empty() && label.len() <= 128, "invalid_label");
        let locks = ProfileStore::open(self.root().join(".locks"))?;
        let _writer = locks.lock(name)?;
        let mut pending = self.prepare_pair(name, inv, replace)?;
        let channel = connect(&inv.endpoint, tls(&inv.lab_id, &inv.ca_pem)).await?;
        info(channel.clone(), &inv.lab_id, &inv.instance, inv.channel).await?;
        let response = pb::pairing_client::PairingClient::new(channel)
            .pair(pb::PairRequest {
                invitation_id: inv.invitation_id.clone(),
                capability: inv.capability.clone(),
                csr_der: pending.record.csr.clone(),
                label: label.into(),
            })
            .await?
            .into_inner();
        ensure!(
            response.lab_id == inv.lab_id && response.channel == inv.channel.as_str(),
            "laboratory_identity_changed"
        );
        let grant = response
            .grant
            .ok_or_else(|| anyhow::anyhow!("invalid_pair_response"))?;
        ensure!(
            grant.access
                == match inv.access {
                    crate::Access::Read => 1,
                    crate::Access::Operate => 2,
                }
                && grant.expires_at == inv.grant_expires_at.map(|v| v as i64)
                && !grant.revoked,
            "invalid_pair_response"
        );
        let certificate_expiry =
            validate_certificate(&pending.record, &response.certificate_pem, &grant.id)?;
        ensure!(
            certificate_expiry == grant.certificate_expires_at,
            "invalid_pair_response"
        );
        pending.record.credential = Credential::Paired {
            certificate: response.certificate_pem,
            grant: grant.id.clone(),
            certificate_expires_at: grant.certificate_expires_at as u64,
        };
        self.write_secret(name, &serde_json::to_vec(&pending.record)?, true)?;
        Ok(grant)
    }
    pub async fn connect(&self, name: &str) -> Result<TransportChannel> {
        let record: Record = serde_json::from_slice(&self.read_secret(name)?)
            .map_err(|_| anyhow::anyhow!("invalid_private_profile"))?;
        let Credential::Paired {
            certificate: cert,
            grant,
            ..
        } = &record.credential
        else {
            anyhow::bail!("pairing_required");
        };
        validate_certificate(&record, cert, grant)?;
        let channel = connect(
            &record.endpoint,
            tls(&record.lab_id, &record.ca_pem).identity(Identity::from_pem(cert, &record.key_pem)),
        )
        .await?;
        info(
            channel.clone(),
            &record.lab_id,
            &record.instance,
            record.channel,
        )
        .await?;
        Ok(channel)
    }
}
fn validate_certificate(record: &Record, pem: &str, grant: &str) -> Result<i64> {
    let (_, leaf) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| anyhow::anyhow!("invalid_certificate"))?;
    let (_, cert) = x509_parser::parse_x509_certificate(&leaf.contents)
        .map_err(|_| anyhow::anyhow!("invalid_certificate"))?;
    let key = rcgen::KeyPair::from_pem(&record.key_pem)?;
    ensure!(
        cert.public_key().raw == key.subject_public_key_info()
            && cert.validity().is_valid()
            && !cert.is_ca()
            && cert
                .subject()
                .iter_common_name()
                .next()
                .and_then(|name| name.as_str().ok())
                == Some(grant),
        "invalid_certificate"
    );
    let eku = cert
        .extended_key_usage()?
        .ok_or_else(|| anyhow::anyhow!("invalid_certificate"))?;
    ensure!(
        eku.value.client_auth && !eku.value.server_auth && !eku.value.any,
        "invalid_certificate"
    );
    let (_, ca) = x509_parser::pem::parse_x509_pem(record.ca_pem.as_bytes())
        .map_err(|_| anyhow::anyhow!("invalid_ca"))?;
    let (_, ca) = x509_parser::parse_x509_certificate(&ca.contents)
        .map_err(|_| anyhow::anyhow!("invalid_ca"))?;
    cert.verify_signature(Some(ca.public_key()))
        .map_err(|_| anyhow::anyhow!("invalid_certificate"))?;
    Ok(cert.validity().not_after.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returned_certificate_is_bound_to_saved_key_and_grant() {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca = rcgen::CertificateParams::default();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        let ca_pem = ca.self_signed(&ca_key).unwrap().pem();
        let issuer = rcgen::Issuer::from_ca_cert_pem(&ca_pem, ca_key).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let mut leaf = rcgen::CertificateParams::default();
        leaf.is_ca = rcgen::IsCa::ExplicitNoCa;
        leaf.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        leaf.distinguished_name
            .push(rcgen::DnType::CommonName, "grant-id");
        let certificate = leaf.signed_by(&key, &issuer).unwrap().pem();
        let mut record = Record {
            lab_id: uuid::Uuid::new_v4().to_string(),
            instance: "test".into(),
            channel: Channel::Stable,
            endpoint: "https://100.64.0.9:50051".into(),
            ca_pem,
            invitation_id: uuid::Uuid::new_v4().to_string(),
            key_pem: key.serialize_pem(),
            csr: Vec::new(),
            access: crate::Access::Read,
            grant_expires_at: None,
            credential: Credential::Pending,
        };
        assert!(validate_certificate(&record, &certificate, "grant-id").is_ok());
        assert!(validate_certificate(&record, &certificate, "another-grant").is_err());
        let temp = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(temp.path().join("profiles")).unwrap();
        let expiry = validate_certificate(&record, &certificate, "grant-id").unwrap() as u64;
        record.credential = Credential::Paired {
            certificate: certificate.clone(),
            grant: "grant-id".into(),
            certificate_expires_at: expiry,
        };
        store
            .write_secret("test", &serde_json::to_vec(&record).unwrap(), false)
            .unwrap();
        let public = store.metadata("test").unwrap();
        assert!(
            matches!(public.state, ProfileState::Paired { grant_id, certificate_expires_at } if grant_id == "grant-id" && certificate_expires_at == expiry)
        );
        record.key_pem = rcgen::KeyPair::generate().unwrap().serialize_pem();
        assert!(validate_certificate(&record, &certificate, "grant-id").is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn failed_pending_publication_prevents_any_pair_connection() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let root = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(root.path().join("profiles")).unwrap();
        store
            .write_secret("blocked", b"previous-private-record", false)
            .unwrap();
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(store.root().join("blocked"))
            .unwrap();
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca = rcgen::CertificateParams::default();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        let invitation = Invitation {
            lab_id: uuid::Uuid::new_v4().to_string(),
            instance: "test".into(),
            channel: Channel::Stable,
            endpoint: "https://100.64.0.9:50051".into(),
            ca_pem: ca.self_signed(&ca_key).unwrap().pem(),
            invitation_id: uuid::Uuid::new_v4().to_string(),
            capability: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
            expires_at: crate::now() + 600,
            access: crate::Access::Read,
            grant_expires_at: None,
        };
        CONNECTION_ATTEMPTS
            .scope(std::cell::Cell::new(0), async {
                let error = store
                    .pair("blocked", &invitation, "test", true)
                    .await
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .starts_with("private_store_native_failure"),
                    "{error}"
                );
                assert_eq!(CONNECTION_ATTEMPTS.with(std::cell::Cell::get), 0);
            })
            .await;
        assert_eq!(
            store.read_secret("blocked").unwrap(),
            b"previous-private-record"
        );
        drop(locked);
    }
}
