//! Per-platform storage layout for OpenSDL.
//!
//! We follow the platform's standard conventions via the `directories`
//! crate (XDG on Linux, Apple's `~/Library/...` on macOS, `%APPDATA%`
//! / `%LOCALAPPDATA%` on Windows) so files land where users /
//! sysadmins / backup tools expect them. The mapping:
//!
//! | Purpose       | Linux ($XDG_*)   | macOS                                    | Windows                                       |
//! | ------------- | ---------------- | ---------------------------------------- | --------------------------------------------- |
//! | config        | CONFIG_HOME      | ~/Library/Application Support/osdl       | %APPDATA%\scienceol\osdl\config               |
//! | state (db)    | STATE_HOME       | ~/Library/Application Support/osdl       | %LOCALAPPDATA%\scienceol\osdl\data            |
//! | cache         | CACHE_HOME       | ~/Library/Caches/com.scienceol.osdl      | %LOCALAPPDATA%\scienceol\osdl\cache           |
//! | runtime       | RUNTIME_DIR      | /tmp/osdl-$UID/run (no Apple equivalent) | %LOCALAPPDATA%\scienceol\osdl\run             |
//!
//! The runtime dir is the trickiest: macOS / Windows have no
//! `XDG_RUNTIME_DIR` analog, so we synthesize a per-user directory
//! under `$TMPDIR` (Unix) or `data_local_dir` (Windows). UDS paths
//! over ~104 chars fail to bind on darwin, so we keep it short.
//! On Windows the runtime dir holds lockfiles only — there are no
//! Unix domain sockets to worry about.

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

#[cfg(unix)]
fn fallback_runtime_dir() -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let uid = current_uid();
    base.join(format!("osdl-{uid}"))
}

/// Windows fallback. `directories::ProjectDirs::runtime_dir()` is `None`
/// on Windows by design, so we co-locate runtime state under
/// `%LOCALAPPDATA%\scienceol\osdl\run` (the same parent as `state_dir`).
/// No need for a UID/username segment — `%LOCALAPPDATA%` is already
/// per-user.
#[cfg(not(unix))]
fn fallback_runtime_dir() -> PathBuf {
    let dirs = directories::ProjectDirs::from(QUALIFIER, ORG, APP);
    match dirs {
        Some(d) => d.data_local_dir().join("run"),
        // Last-resort: `directories` couldn't find a home. Fall back to
        // the env var directly. `LOCALAPPDATA` is set on every modern
        // Windows install; if it isn't, we have bigger problems.
        None => {
            let base = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            base.join("scienceol").join("osdl").join("run")
        }
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid is always-safe.
    unsafe { libc::getuid() }
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
