//! Server-owned display settings, independent of engine configuration.

use crate::authorization::lease::Resource;
use anyhow::{ensure, Context};
use osdl_proto::v1 as pb;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;
use tonic::Status;
use uuid::Uuid;

#[cfg(test)]
mod tests;

const MAX_PROFILE_BYTES: u64 = 4096;
const MAX_NAME_CHARS: usize = 128;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    name: String,
    description: String,
    revision: String,
}

impl Profile {
    fn response(self, writable: bool) -> pb::LabProfile {
        pb::LabProfile {
            name: self.name,
            description: self.description,
            revision: self.revision,
            writable,
        }
    }
}

pub(crate) struct Store {
    path: PathBuf,
    // Store clones and in-flight publications retain this exclusive ownership.
    _ownership: File,
    profile: RwLock<Profile>,
    writer: Arc<Mutex<()>>,
}

impl Store {
    pub(crate) fn open(
        dir: &Path,
        instance: &str,
        initial_name: Option<&str>,
    ) -> anyhow::Result<Self> {
        // The digest preserves case-distinct instance identities on every filesystem
        // and keeps both filenames below the maximum component length.
        let path = dir
            .canonicalize()?
            .join(format!("lab-profile-{:x}.json", Sha256::digest(instance)));
        let ownership = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        ownership
            .try_lock()
            .context("acquire laboratory profile ownership")?;
        let profile = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= MAX_PROFILE_BYTES,
                    "profile file is too large"
                );
                let profile: Profile =
                    serde_json::from_slice(&bytes).context("invalid profile file")?;
                let revision =
                    Uuid::parse_str(&profile.revision).context("invalid profile revision")?;
                ensure!(
                    !revision.is_nil() && profile.revision == revision.to_string(),
                    "invalid profile revision"
                );
                validate(&profile.name, &profile.description)?;
                profile
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = match initial_name {
                    Some(name) => name.to_owned(),
                    None => hostname_display_name(&hostname().unwrap_or_default(), instance),
                };
                validate(&name, "")?;
                Profile {
                    name: name.trim().to_owned(),
                    description: String::new(),
                    revision: Uuid::new_v4().to_string(),
                }
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            _ownership: ownership,
            profile: RwLock::new(profile),
            writer: Arc::new(Mutex::new(())),
        })
    }

    fn current(&self) -> Profile {
        self.profile.read().unwrap().clone()
    }

    pub(crate) fn snapshot(&self, writable: bool) -> pb::LabProfile {
        self.current().response(writable)
    }

    pub(crate) async fn update(
        self: Arc<Self>,
        request: pb::UpdateLabProfileRequest,
        authority: Option<Resource>,
    ) -> Result<pb::LabProfile, Status> {
        validate(&request.name, &request.description)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        let name = request.name.trim();
        let writer = self.writer.clone().lock_owned().await;
        let current = self.current();
        if current.revision != request.revision {
            return Err(Status::aborted("laboratory profile changed"));
        }
        if current.name == name && current.description == request.description {
            return Ok(current.response(true));
        }
        let next = Profile {
            name: name.to_owned(),
            description: request.description,
            revision: Uuid::new_v4().to_string(),
        };
        tokio::task::spawn_blocking(move || {
            let _writer = writer;
            // The blocking write can outlive a cancelled RPC. Keep its authorization
            // resource here and check its deadline once the write can begin.
            if let Some(lease) = &authority {
                lease.check_current()?;
            }
            let updated = self.persist(next)?;
            if let Some(lease) = &authority {
                lease.audit_account("update_lab_profile", &updated.revision);
            }
            Ok(updated)
        })
        .await
        .map_err(|_| Status::internal("could not save laboratory profile"))?
    }

    fn persist(&self, next: Profile) -> Result<pb::LabProfile, Status> {
        let directory_sync_error = self.publish(&next).map_err(|error| {
            log::error!("cannot persist laboratory profile: {error:#}");
            Status::internal("could not save laboratory profile")
        })?;
        *self.profile.write().unwrap() = next.clone();
        if let Some(error) = directory_sync_error {
            log::error!("laboratory profile published but directory sync failed: {error}");
            return Err(Status::unknown(
                "laboratory profile durability is uncertain",
            ));
        }
        Ok(next.response(true))
    }

    /// A post-publication directory-flush error cannot undo the replacement.
    fn publish(&self, profile: &Profile) -> anyhow::Result<Option<io::Error>> {
        let parent = self.path.parent().expect("profile has a parent directory");
        #[cfg(unix)]
        let directory = File::open(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&serde_json::to_vec(profile)?)?;
        #[cfg(unix)]
        replace(temp, &self.path)?;
        #[cfg(windows)]
        osdl_auth::publish_windows_file(temp, &self.path, true)?;
        #[cfg(unix)]
        let directory_sync_error = directory.sync_all().err();
        #[cfg(windows)]
        let directory_sync_error = None;
        Ok(directory_sync_error)
    }
}

fn validate(name: &str, description: &str) -> anyhow::Result<()> {
    ensure!(
        name.chars().take(MAX_NAME_CHARS + 1).count() <= MAX_NAME_CHARS
            && !name.trim().is_empty()
            && !name
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        "name must be a single line of 1 to 128 characters",
    );
    ensure!(
        description.chars().take(501).count() <= 500
            && !description
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
        "description must be at most 500 characters of text",
    );
    Ok(())
}

fn hostname_display_name(host: &str, instance: &str) -> String {
    let bounded = host.trim().chars().take(MAX_NAME_CHARS).collect::<String>();
    let name = bounded.trim_end();
    if validate(name, "").is_ok() {
        name.to_owned()
    } else {
        instance.to_owned()
    }
}

#[cfg(unix)]
fn hostname() -> io::Result<String> {
    let mut raw = [0_u8; 256];
    // SAFETY: The output is a writable buffer of the size passed to gethostname.
    if unsafe { libc::gethostname(raw.as_mut_ptr().cast(), raw.len()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let end = raw.iter().position(|&byte| byte == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
}

#[cfg(windows)]
fn hostname() -> io::Result<String> {
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNamePhysicalDnsHostname, GetComputerNameExW,
    };

    let mut raw = [0_u16; 256];
    let mut len = raw.len() as u32;
    // SAFETY: The output is a writable UTF-16 buffer and len points to its capacity.
    if unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, raw.as_mut_ptr(), &mut len) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf16_lossy(&raw[..len as usize]))
}

#[cfg(unix)]
fn replace(temp: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}
