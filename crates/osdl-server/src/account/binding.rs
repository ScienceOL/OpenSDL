use anyhow::{ensure, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub issuer: String,
    pub channel: String,
    pub account_id: String,
    pub lab_id: String,
    pub instance: String,
    pub key_id: String,
    pub public_key: String,
}
pub fn channel_for_origin(origin: &str) -> anyhow::Result<&'static str> {
    match origin {
        "https://liyanlabs.local" => Ok("dev"),
        "https://test.liyanlabs.com" => Ok("beta"),
        "https://uat.liyanlabs.com" => Ok("rc"),
        "https://liyanlabs.com" => Ok("stable"),
        _ => anyhow::bail!("server must be a canonical LiyanLabs HTTPS origin"),
    }
}
pub(super) fn uuid(value: &str) -> anyhow::Result<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(value).context("invalid identity UUID")?;
    ensure!(
        !id.is_nil() && id.to_string() == value,
        "invalid identity UUID"
    );
    Ok(id)
}
impl Binding {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            channel_for_origin(&self.issuer)? == self.channel,
            "binding environment mismatch"
        );
        ensure!(
            !self.account_id.is_empty()
                && self.account_id.len() <= 256
                && self.account_id.trim() == self.account_id,
            "invalid binding account"
        );
        uuid(&self.lab_id)?;
        ensure!(
            !self.instance.is_empty()
                && self.instance.len() <= 128
                && self.instance.trim() == self.instance
                && !self.instance.chars().any(char::is_control),
            "invalid instance"
        );
        ensure!(
            !self.key_id.is_empty()
                && self.key_id.len() <= 256
                && self.key_id.trim() == self.key_id,
            "invalid verification key ID"
        );
        ensure!(
            B64.decode(&self.public_key)
                .is_ok_and(|key| key.len() == 32),
            "invalid verification key"
        );
        Ok(())
    }
}

impl Binding {
    pub fn path(config: &Path, instance: &str) -> anyhow::Result<PathBuf> {
        ensure!(
            !instance.is_empty()
                && instance.len() <= 128
                && instance
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
            "instance must contain only letters, digits, '-' or '_'"
        );
        Ok(config.join("instances").join(instance).join("account.json"))
    }
    /// Atomic create-only publication preserves laboratory identity even across concurrent logins.
    pub fn save_new(&self, config: &Path) -> anyhow::Result<()> {
        use std::io::Write;
        self.validate()?;
        let path = Self::path(config, &self.instance)?;
        let directory = path.parent().expect("binding parent");
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&path).map_err(|_| {
            anyhow::anyhow!("instance already bound or binding could not be published")
        })?;
        #[cfg(unix)]
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
    pub fn load(config: &Path, instance: &str) -> anyhow::Result<Self> {
        use std::io::Read;
        let mut data = Vec::new();
        std::fs::File::open(Self::path(config, instance)?)
            .context("account binding required; run lab login")?
            .take(8193)
            .read_to_end(&mut data)?;
        ensure!(data.len() <= 8192, "binding too large");
        let binding: Self = serde_json::from_slice(&data)?;
        binding.validate()?;
        ensure!(binding.instance == instance, "binding instance mismatch");
        Ok(binding)
    }
}
