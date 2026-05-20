//! Per-instance lockfile for multi-server discovery.
//!
//! Inspired by VS Code's per-window socket files: each `osdl serve` writes
//! a JSON descriptor to `runtime_dir/instances/<NAME>.json`. Clients scan
//! this directory to discover running servers, skipping entries whose PID
//! is no longer alive.
//!
//! Why JSON-per-instance instead of one shared file: a shared file would
//! need locking on read/write, and one server crashing mid-write could
//! corrupt the index for everyone. A per-instance file isolates failure
//! and lets the filesystem itself act as the registry.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// JSON record persisted while the server is running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub instance: String,
    pub pid: u32,
    pub version: String,
    pub started_at_unix_ms: u64,
    /// Unix domain socket path, if listening on UDS.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub socket_path: Option<PathBuf>,
    /// `host:port` if listening on TCP.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub listen_addr: Option<String>,
}

impl InstanceRecord {
    pub fn new(
        instance: impl Into<String>,
        pid: u32,
        version: impl Into<String>,
        socket_path: Option<PathBuf>,
        listen_addr: Option<String>,
    ) -> Self {
        let started_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            instance: instance.into(),
            pid,
            version: version.into(),
            started_at_unix_ms,
            socket_path,
            listen_addr,
        }
    }
}

/// RAII guard that deletes the lockfile on drop. Held by `serve()` for
/// the lifetime of the server.
pub struct LockfileGuard {
    path: PathBuf,
}

impl LockfileGuard {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockfileGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            // ENOENT is fine — someone else already cleaned up.
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("failed to remove lockfile {}: {}", self.path.display(), e);
            }
        }
    }
}

/// Write a fresh lockfile for the running instance.
///
/// Refuses to clobber an existing live lockfile (same instance name with a
/// running PID); a stale one (PID dead) is overwritten.
///
/// Creation is atomic: we use `OpenOptions::create_new` (POSIX `O_EXCL`)
/// so two processes racing to start the same instance cannot both
/// succeed. The loser observes EEXIST, re-parses the file, and either
/// reports the live winner or — if it finds a stale entry — removes and
/// retries exactly once. A second EEXIST means a third process won the
/// race; bail cleanly rather than spin.
pub fn write(lock_dir: &Path, record: &InstanceRecord) -> Result<LockfileGuard, String> {
    use std::io::Write;

    std::fs::create_dir_all(lock_dir)
        .map_err(|e| format!("create lock dir {}: {}", lock_dir.display(), e))?;

    let final_path = lock_dir.join(format!("{}.json", sanitize(&record.instance)));
    let json = serde_json::to_vec_pretty(record)
        .map_err(|e| format!("serialize record: {e}"))?;

    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&final_path)
        {
            Ok(mut f) => {
                f.write_all(&json)
                    .map_err(|e| format!("write {}: {}", final_path.display(), e))?;
                return Ok(LockfileGuard { path: final_path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Inspect the existing file. If the recorded PID is alive,
                // refuse — we're not the only legitimate owner.
                if let Some(existing) = read_record(&final_path) {
                    if pid_alive(existing.pid) {
                        return Err(format!(
                            "instance '{}' already running (pid {} at {})",
                            existing.instance,
                            existing.pid,
                            existing
                                .socket_path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .or(existing.listen_addr.clone())
                                .unwrap_or_else(|| "<unknown>".into()),
                        ));
                    }
                }
                // Stale (or unparseable). On the first attempt, remove and
                // retry. On the second, another process beat us to creating
                // a fresh lock — error out so we don't loop forever.
                if attempt == 0 {
                    let _ = std::fs::remove_file(&final_path);
                    continue;
                }
                return Err(format!(
                    "lockfile {} race: another process won the create after stale cleanup",
                    final_path.display()
                ));
            }
            Err(e) => {
                return Err(format!("create {}: {}", final_path.display(), e));
            }
        }
    }
    // Unreachable because the loop body always returns.
    Err("lockfile write loop exited unexpectedly".into())
}

/// All live instances under `lock_dir`. Stale entries (dead PID) are
/// returned — and also cleaned up from disk as a side effect — so callers
/// don't have to re-implement that logic.
pub fn list(lock_dir: &Path) -> Vec<InstanceRecord> {
    let entries = match std::fs::read_dir(lock_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(rec) = read_record(&path) else { continue };
        if pid_alive(rec.pid) {
            out.push(rec);
        } else {
            // Best-effort GC — failure here is fine, we just skip the entry.
            let _ = std::fs::remove_file(&path);
        }
    }
    out.sort_by(|a, b| a.instance.cmp(&b.instance));
    out
}

/// Look up a single instance by name. Returns `None` for missing or stale.
pub fn find(lock_dir: &Path, instance: &str) -> Option<InstanceRecord> {
    let path = lock_dir.join(format!("{}.json", sanitize(instance)));
    let rec = read_record(&path)?;
    if pid_alive(rec.pid) {
        Some(rec)
    } else {
        let _ = std::fs::remove_file(&path);
        None
    }
}

fn read_record(path: &Path) -> Option<InstanceRecord> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Strip path separators and other meta-chars from an instance name so it
/// can be safely used as a filename component. Matches what we accept on
/// the CLI: `[A-Za-z0-9._-]+`.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // kill(pid, 0) returns 0 if the process exists and we have permission
    // to signal it; ESRCH means it's gone. EPERM means it exists but
    // belongs to another user — treat that as alive (something is there).
    if pid == 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    err.raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    // No portable check; conservatively assume alive. Windows support is
    // out of scope for the initial cut.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_and_find_round_trip() {
        let dir = tempdir().unwrap();
        let rec = InstanceRecord::new(
            "default",
            std::process::id(),
            "test",
            Some(dir.path().join("default.sock")),
            None,
        );
        let _guard = write(dir.path(), &rec).unwrap();
        let found = find(dir.path(), "default").expect("should find");
        assert_eq!(found.instance, "default");
        assert_eq!(found.pid, std::process::id());
    }

    #[test]
    fn list_skips_stale() {
        let dir = tempdir().unwrap();
        // Live entry.
        let live = InstanceRecord::new("live", std::process::id(), "test", None, None);
        let _g = write(dir.path(), &live).unwrap();

        // Stale entry: pick a PID that's almost certainly dead. PID 1 is
        // typically init (and alive), so use a high number that's
        // unlikely to be running. We also can't *guarantee* it's dead on
        // a busy system, so we accept a small flake risk for a test that
        // exercises the GC path.
        let stale = InstanceRecord::new("stale", 999_999, "test", None, None);
        let stale_path = dir.path().join("stale.json");
        std::fs::write(&stale_path, serde_json::to_vec(&stale).unwrap()).unwrap();

        let entries = list(dir.path());
        assert!(entries.iter().any(|e| e.instance == "live"));
        assert!(entries.iter().all(|e| e.instance != "stale"));
    }

    #[test]
    fn write_refuses_to_clobber_live_lock() {
        let dir = tempdir().unwrap();
        let rec = InstanceRecord::new("default", std::process::id(), "test", None, None);
        let _g = write(dir.path(), &rec).unwrap();
        let again = write(dir.path(), &rec);
        assert!(again.is_err());
    }

    #[test]
    fn sanitize_strips_path_traversal() {
        assert_eq!(sanitize("../etc/passwd"), ".._etc_passwd");
        assert_eq!(sanitize("normal-name_1"), "normal-name_1");
    }
}
