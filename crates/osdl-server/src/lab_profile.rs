//! Server-owned display settings, independent of engine configuration.

use anyhow::{ensure, Context};
use osdl_proto::v1 as pb;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;
use tonic::Status;
use uuid::Uuid;

#[cfg(test)]
mod tests;

const MAX_PROFILE_BYTES: u64 = 4096;

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
    profile: RwLock<Profile>,
    writer: Arc<Mutex<()>>,
}

impl Store {
    pub(crate) fn open(
        dir: &Path,
        instance: &str,
        initial_name: Option<&str>,
    ) -> anyhow::Result<Self> {
        let path = dir
            .canonicalize()?
            .join(format!("{instance}.lab-profile.json"));
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
                    None => hostname().context("resolve host name")?,
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
            self.persist(next).map_err(|error| {
                log::error!("cannot persist laboratory profile: {error:#}");
                Status::internal("could not save laboratory profile")
            })
        })
        .await
        .map_err(|_| Status::internal("could not save laboratory profile"))?
    }

    fn persist(&self, next: Profile) -> anyhow::Result<pb::LabProfile> {
        let parent = self.path.parent().expect("profile has a parent directory");
        #[cfg(unix)]
        let directory = File::open(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&serde_json::to_vec(&next)?)?;
        replace(temp, &self.path)?;
        #[cfg(unix)]
        let sync_result = directory.sync_all();

        // A failed directory flush is reported to the caller, but the rename
        // already made this revision visible on disk.
        *self.profile.write().unwrap() = next.clone();
        #[cfg(unix)]
        sync_result?;
        Ok(next.response(true))
    }
}

fn validate(name: &str, description: &str) -> anyhow::Result<()> {
    ensure!(
        name.chars().take(129).count() <= 128
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

#[cfg(windows)]
fn replace(mut temp: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH,
    };

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let source = wide(temp.path());
    let destination = wide(path);
    // SAFETY: Both paths are NUL-terminated UTF-16 arrays valid for these calls.
    if unsafe { SetFileAttributesW(source.as_ptr(), FILE_ATTRIBUTE_NORMAL) } == 0 {
        return Err(io::Error::last_os_error());
    }
    temp.as_file().sync_all()?;
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    temp.disable_cleanup(true);
    Ok(())
}
