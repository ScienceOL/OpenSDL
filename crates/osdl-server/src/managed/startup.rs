use super::store::Store;
use std::process::Child;

/// The initiating future owns the process until authenticated readiness.
/// Cancellation drops this guard and cannot abandon an untracked child.
pub(super) struct Startup {
    pub child: Option<Child>,
    pub store: Store,
    pub name: String,
    pub generation: i64,
}
impl Startup {
    pub fn child(&mut self) -> &mut Child {
        self.child.as_mut().expect("startup owns its child")
    }
    pub fn ready(&mut self) {
        self.child.take();
    }
}
impl Drop for Startup {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            if let Err(error) = child.wait() {
                log::error!("cannot reap interrupted instance startup: {error}");
            }
            if let Err(error) = self.store.settle(
                &self.name,
                self.generation,
                "failed",
                Some("startup interrupted or failed; inspect retained logs"),
            ) {
                log::error!("cannot settle interrupted instance startup: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensdl_assets::AssetCache;
    use std::process::{Command, Stdio};

    #[test]
    fn startup_test_process_waits() {
        if let Some(path) = std::env::var_os("LAB_TEST_STARTUP_FILE") {
            std::fs::write(&path, b"started").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(5));
            std::fs::write(path, b"finished").unwrap();
        }
    }
    #[test]
    fn interrupted_startup_reaps_its_child_and_retains_a_failed_record() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(AssetCache::open(temp.path().join("store")).unwrap()).unwrap();
        store
            .reserve("demo", &format!("sha256:{}", "a".repeat(64)))
            .unwrap();
        let generation = store.generation("demo", "private").unwrap();
        let marker = temp.path().join("child.txt");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "managed::startup::tests::startup_test_process_waits",
            ])
            .env("LAB_TEST_STARTUP_FILE", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let guard = Startup {
            child: Some(child),
            store: store.clone(),
            name: "demo".into(),
            generation,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "test child did not start");
        drop(guard);
        assert_eq!(std::fs::read(marker).unwrap(), b"started");
        let record = store.read("demo").unwrap();
        assert!(matches!(
            record.info.state,
            super::super::InstanceState::Failed
        ));
        assert!(record.token.is_empty());
    }
}
