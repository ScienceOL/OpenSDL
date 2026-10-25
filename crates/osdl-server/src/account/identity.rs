use super::{binding, Binding};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;
use tonic::Status;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Scope {
    #[serde(rename = "opensdl:read")]
    Read,
    #[serde(rename = "opensdl:read opensdl:operate")]
    Operate,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Principal {
    pub device_id: uuid::Uuid,
    pub device_generation: u64,
    pub installation_id: uuid::Uuid,
}
#[derive(Clone, Debug)]
pub struct Identity {
    pub principal: Principal,
    pub scope: Scope,
    pub expires_at: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    typ: String,
    kid: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    iss: String,
    sub: String,
    aud: String,
    channel: String,
    device_id: String,
    device_generation: u64,
    installation_id: String,
    scope: Scope,
    iat: u64,
    exp: u64,
}
#[derive(Clone, Debug)]
pub struct Verifier {
    binding: Binding,
    key: Vec<u8>,
}
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl Verifier {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn new(binding: Binding) -> anyhow::Result<Self> {
        binding.validate()?;
        let key = B64.decode(&binding.public_key)?;
        Ok(Self { binding, key })
    }
    pub fn verify(&self, token: &str) -> Result<Identity, Status> {
        let c = self
            .signed_claims(token)
            .map_err(|_| AuthError::InvalidAssertion.status())?;
        if c.iss != self.binding.issuer
            || c.aud != self.binding.lab_id
            || c.channel != self.binding.channel
        {
            return Err(AuthError::InvalidAssertion.status());
        }
        let now = now();
        if c.iat > now || c.exp <= c.iat || c.exp - c.iat > 300 || c.device_generation == 0 {
            return Err(AuthError::InvalidAssertion.status());
        }
        let principal = Principal {
            device_id: binding::uuid(&c.device_id)
                .map_err(|_| AuthError::InvalidAssertion.status())?,
            device_generation: c.device_generation,
            installation_id: binding::uuid(&c.installation_id)
                .map_err(|_| AuthError::InvalidAssertion.status())?,
        };
        // Account/expiry diagnostics are trusted only after signature and shape validation.
        if c.sub != self.binding.account_id {
            return Err(AuthError::AccountMismatch.status());
        }
        if c.exp <= now {
            return Err(AuthError::Expired.status());
        }
        Ok(Identity {
            principal,
            scope: c.scope,
            expires_at: c.exp,
        })
    }
    fn signed_claims(&self, token: &str) -> anyhow::Result<Claims> {
        anyhow::ensure!(token.len() <= 4096, "token too large");
        let (signed, signature) = token
            .rsplit_once('.')
            .ok_or_else(|| anyhow::anyhow!("invalid JWT"))?;
        let (header, payload) = signed
            .split_once('.')
            .ok_or_else(|| anyhow::anyhow!("invalid JWT"))?;
        let h: Header = serde_json::from_slice(&B64.decode(header)?)?;
        anyhow::ensure!(
            h.alg == "EdDSA"
                && h.typ == "liyanlabs.opensdl.identity+jwt"
                && h.kid == self.binding.key_id,
            "invalid JWT header"
        );
        UnparsedPublicKey::new(&ED25519, &self.key)
            .verify(signed.as_bytes(), &B64.decode(signature)?)
            .map_err(|_| anyhow::anyhow!("invalid signature"))?;
        Ok(serde_json::from_slice(&B64.decode(payload)?)?)
    }
    pub fn authenticate<T>(&self, request: &tonic::Request<T>) -> Result<Identity, Status> {
        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| AuthError::InvalidAssertion.status())?;
        self.verify(token)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AuthError {
    InvalidAssertion,
    AccountMismatch,
    Expired,
}
impl AuthError {
    pub(crate) fn status(self) -> Status {
        let (code, message) = match self {
            Self::InvalidAssertion => ("invalid_assertion", "invalid laboratory authorization"),
            Self::AccountMismatch => ("account_mismatch", "laboratory account mismatch"),
            Self::Expired => ("authorization_expired", "laboratory authorization expired"),
        };
        let mut status = Status::unauthenticated(message);
        status.metadata_mut().insert(
            "opensdl-auth-error",
            tonic::metadata::MetadataValue::from_static(code),
        );
        status
    }
}
