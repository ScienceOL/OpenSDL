//! Per-platform storage layout for OpenSDL.
//!
//! We follow the platform's standard conventions via the `directories`
//! crate (XDG on Linux, Apple's `~/Library/...` on macOS) so files land
//! where users / sysadmins / backup tools expect them. The mapping:
//!
//! | Purpose       | Linux ($XDG_*)   | macOS                                  |
//! | ------------- | ---------------- | -------------------------------------- |
//! | config        | CONFIG_HOME      | ~/Library/Application Support/osdl     |
//! | state (db)    | STATE_HOME       | ~/Library/Application Support/osdl     |
//! | cache         | CACHE_HOME       | ~/Library/Caches/com.scienceol.osdl    |
//! | runtime (sock)| RUNTIME_DIR      | /tmp/osdl-$UID/run (no Apple equivalent) |
//!
//! The runtime dir is the trickiest: macOS has no `XDG_RUNTIME_DIR` analog,
//! so we synthesize a per-UID directory under `$TMPDIR` (or `/tmp`). UDS
//! paths over ~104 chars fail to bind on darwin, so we keep it short.

use directories::ProjectDirs;
use std::path::PathBuf;

const QUALIFIER: &str = "com";
const ORG: &str = "scienceol";
const APP: &str = "osdl";

/// Resolved on-disk paths for a particular invocation.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl Paths {
    /// Compute default paths for the current user, creating directories
    /// as needed. The runtime dir is created with mode 0700 on Unix so
    /// other users on the host can't connect to our sockets.
    pub fn discover() -> Result<Self, String> {
        let dirs = ProjectDirs::from(QUALIFIER, ORG, APP)
            .ok_or_else(|| "no valid home directory for current user".to_string())?;

        let config_dir = dirs.config_dir().to_path_buf();
        let state_dir = dirs
            .state_dir()
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs.data_local_dir().to_path_buf());
        let cache_dir = dirs.cache_dir().to_path_buf();

        // Runtime dir: prefer XDG_RUNTIME_DIR, fall back to a per-UID
        // tempdir that we own.
        let runtime_dir = match dirs.runtime_dir() {
            Some(p) => p.to_path_buf(),
            None => fallback_runtime_dir(),
        };

        for d in [&config_dir, &state_dir, &cache_dir, &runtime_dir] {
            std::fs::create_dir_all(d)
                .map_err(|e| format!("create {}: {}", d.display(), e))?;
        }

        // Tighten runtime dir perms (Unix only). The XDG-supplied dir is
        // already 0700; the fallback dir we just created might be 0755.
        #[cfg(unix)]
        tighten_perms(&runtime_dir, 0o700)?;

        Ok(Self {
            config_dir,
            state_dir,
            cache_dir,
            runtime_dir,
        })
    }

    /// Where instance lockfiles live. One JSON file per running server.
    pub fn lock_dir(&self) -> PathBuf {
        self.runtime_dir.join("instances")
    }

    /// Default path for the SQLite event store (one DB per instance, so
    /// callers can override when running multiple instances side-by-side).
    pub fn default_db_path(&self, instance: &str) -> PathBuf {
        self.state_dir.join(format!("{instance}.db"))
    }

    /// Default UDS path for the named instance. Kept short on macOS where
    /// the sun_path limit is ~104 bytes.
    pub fn default_socket_path(&self, instance: &str) -> PathBuf {
        self.runtime_dir.join(format!("{instance}.sock"))
    }
}

fn fallback_runtime_dir() -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let uid = current_uid();
    base.join(format!("osdl-{uid}"))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid is always-safe.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

#[cfg(unix)]
fn tighten_perms(path: &std::path::Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, perms)
        .map_err(|e| format!("chmod {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_discover_creates_dirs() {
        let p = Paths::discover().expect("paths");
        assert!(p.config_dir.exists());
        assert!(p.state_dir.exists());
        assert!(p.cache_dir.exists());
        assert!(p.runtime_dir.exists());
    }
}
