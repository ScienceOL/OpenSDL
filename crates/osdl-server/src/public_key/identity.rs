use anyhow::{ensure, Result};
use osdl_auth::{Channel, ProfileStore};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub lab_id: String,
    pub instance: String,
    pub channel: Channel,
    pub ca: String,
    ca_key: String,
    pub certificate: String,
    key: String,
}
impl Identity {
    pub fn load(store: &ProfileStore, instance: &str, channel: Channel) -> Result<Self> {
        if store.contains("identity")? {
            let identity: Self = serde_json::from_slice(&store.read_secret("identity")?)
                .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
            ensure!(
                identity.instance == instance && identity.channel == channel,
                "laboratory_identity_changed"
            );
            identity.validate()?;
            return Ok(identity);
        }
        ensure!(
            !store.root().join("grants.sqlite").exists(),
            "laboratory_identity_missing"
        );
        let lab_id = uuid::Uuid::new_v4().to_string();
        let ca_key = KeyPair::generate()?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, format!("OpenSDL {lab_id}"));
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365 * 30);
        let ca = params.self_signed(&ca_key)?.pem();
        let issuer = Issuer::from_ca_cert_pem(&ca, &ca_key)?;
        let key = KeyPair::generate()?;
        let mut server = CertificateParams::new(vec![format!("lab-{lab_id}.opensdl.invalid")])?;
        server.not_before = params.not_before;
        server.not_after = params.not_after;
        server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        server.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let certificate = server.signed_by(&key, &issuer)?.pem();
        let identity = Self {
            lab_id,
            instance: instance.into(),
            channel,
            ca,
            ca_key: ca_key.serialize_pem(),
            certificate,
            key: key.serialize_pem(),
        };
        store.write_secret("identity", &serde_json::to_vec(&identity)?, false)?;
        Ok(identity)
    }
    fn validate(&self) -> Result<()> {
        use rcgen::PublicKeyData;
        ensure!(
            uuid::Uuid::parse_str(&self.lab_id)?.to_string() == self.lab_id,
            "laboratory_identity_corrupt"
        );
        let (_, ca_pem) = x509_parser::pem::parse_x509_pem(self.ca.as_bytes())
            .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
        let (_, ca) = x509_parser::parse_x509_certificate(&ca_pem.contents)
            .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
        let (_, server_pem) = x509_parser::pem::parse_x509_pem(self.certificate.as_bytes())
            .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
        let (_, server) = x509_parser::parse_x509_certificate(&server_pem.contents)
            .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
        ensure!(
            ca.is_ca()
                && ca.validity().is_valid()
                && server.validity().is_valid()
                && !server.is_ca(),
            "laboratory_identity_corrupt"
        );
        ensure!(
            ca.public_key().raw == KeyPair::from_pem(&self.ca_key)?.subject_public_key_info()
                && server.public_key().raw
                    == KeyPair::from_pem(&self.key)?.subject_public_key_info(),
            "laboratory_identity_corrupt"
        );
        ca.verify_signature(None)
            .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
        server
            .verify_signature(Some(ca.public_key()))
            .map_err(|_| anyhow::anyhow!("laboratory_identity_corrupt"))?;
        let name = format!("lab-{}.opensdl.invalid", self.lab_id);
        ensure!(server.subject_alternative_name()?.is_some_and(|san|san.value.general_names.iter().any(|n|matches!(n,x509_parser::extensions::GeneralName::DNSName(value) if *value==name))),"laboratory_identity_corrupt");
        ensure!(
            server
                .extended_key_usage()?
                .is_some_and(|usage| usage.value.server_auth
                    && !usage.value.any
                    && !usage.value.client_auth),
            "laboratory_identity_corrupt"
        );
        Ok(())
    }
    pub fn issuer(&self) -> Result<Issuer<'static, KeyPair>> {
        Ok(Issuer::from_ca_cert_pem(
            &self.ca,
            KeyPair::from_pem(&self.ca_key)?,
        )?)
    }
    pub fn tls(&self) -> tonic::transport::ServerTlsConfig {
        osdl_auth::crypto_provider();
        tonic::transport::ServerTlsConfig::new()
            .identity(tonic::transport::Identity::from_pem(
                &self.certificate,
                &self.key,
            ))
            .client_ca_root(tonic::transport::Certificate::from_pem(&self.ca))
            .client_auth_optional(true)
            .timeout(std::time::Duration::from_secs(10))
    }
    pub fn issue(&self, csr: &[u8], id: &str, expiry: u64) -> Result<(String, Vec<u8>)> {
        ensure!(csr.len() <= 4096, "invalid_csr");
        use x509_parser::prelude::FromDer;
        let (remaining, _) =
            x509_parser::certification_request::X509CertificationRequest::from_der(csr)
                .map_err(|_| anyhow::anyhow!("invalid_csr"))?;
        ensure!(remaining.is_empty(), "invalid_csr");
        let mut request = rcgen::CertificateSigningRequestParams::from_der(&csr.into())
            .map_err(|_| anyhow::anyhow!("invalid_csr"))?;
        ensure!(
            request.public_key.algorithm() == &rcgen::PKCS_ECDSA_P256_SHA256,
            "unsupported_csr_algorithm"
        );
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::ExplicitNoCa;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.distinguished_name.push(DnType::CommonName, id);
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
        params.not_after = time::OffsetDateTime::from_unix_timestamp(expiry as i64)?;
        request.params = params;
        let cert = request.signed_by(&self.issuer()?)?;
        Ok((cert.pem(), cert.der().to_vec()))
    }
}
