mod support;
use serde_json::Value;
use std::{path::Path, process::Output, time::Duration};

fn lab(store: &Path, cwd: &Path, args: &[&str]) -> Output {
    eprintln!("lab {args:?} in {}", cwd.display());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_lab"))
                .arg("--cache-dir")
                .arg(store)
                .args(args)
                .current_dir(cwd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
                .await
                .unwrap_or_else(|_| panic!("lab timed out: {args:?}"))
                .unwrap()
        })
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
struct DaemonGuard<'a> {
    store: &'a Path,
    cwd: &'a Path,
}
impl Drop for DaemonGuard<'_> {
    fn drop(&mut self) {
        let _ = lab(self.store, self.cwd, &["daemon", "stop", "--json"]);
    }
}

#[test]
fn installed_process_supports_other_workspaces_and_offline_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = root.join("store");
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    support::create_asset(&workspace.join("asset"));
    let started = success(lab(&store, root, &["daemon", "start", "--json"]));
    let _guard = DaemonGuard {
        store: &store,
        cwd: root,
    };
    // The detached child must not keep parent's capture pipes open (Windows regression).
    assert!(started["pid"].as_u64().unwrap() > 0);
    assert!(started.get("token").is_none());
    let built = success(lab(
        &store,
        &workspace,
        &[
            "build",
            "asset",
            "--output",
            "layout",
            "--tag",
            "localhost/lab/demo:1.0.0",
            "--json",
        ],
    ));
    let digest = built["manifestDigest"].as_str().unwrap();
    assert!(workspace.join("layout/oci-layout").is_file());
    assert_eq!(
        success(lab(
            &store,
            &workspace,
            &["inspect", "localhost/lab/demo:1.0.0", "--json"]
        ))["manifestDigest"],
        digest
    );
    let pulled = lab(
        &store,
        &workspace,
        &[
            "pull",
            "localhost/lab/demo:1.0.0",
            "--offline",
            "--output",
            "edited",
        ],
    );
    assert!(
        pulled.status.success(),
        "{}",
        String::from_utf8_lossy(&pulled.stderr)
    );
    assert!(workspace.join("edited/asset.yaml").is_file());
    std::fs::write(
        workspace.join("edited/scene.usda"),
        b"#usda 1.0\ndef Xform \"Updated\" {}\n",
    )
    .unwrap();
    let rebuilt = success(lab(
        &store,
        &workspace,
        &[
            "build",
            "edited",
            "--tag",
            "localhost/lab/demo:2.0.0",
            "--json",
        ],
    ));
    assert_ne!(rebuilt["manifestDigest"], digest);
    assert_eq!(
        success(lab(&store, root, &["images", "--json"]))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    success(lab(&store, root, &["daemon", "stop", "--json"]));
    assert_eq!(
        success(lab(&store, root, &["daemon", "status", "--json"]))["running"],
        false
    );
    assert_eq!(
        success(lab(&store, root, &["inspect", digest, "--json"]))["manifestDigest"],
        digest
    );
}

#[test]
fn duplicate_foreground_daemon_exits_and_stop_allows_immediate_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = root.join("store");
    let first = success(lab(&store, root, &["daemon", "start", "--json"]));
    let _guard = DaemonGuard {
        store: &store,
        cwd: root,
    };
    let duplicate = lab(&store, root, &["daemon", "start", "--foreground"]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already running"));
    success(lab(&store, root, &["daemon", "stop", "--json"]));
    let second = success(lab(&store, root, &["daemon", "start", "--json"]));
    assert_ne!(first["pid"], second["pid"]);
}

#[test]
fn stale_unresponsive_daemon_does_not_silently_fall_back_to_direct_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = root.join("store");
    let _cache = opensdl_assets::AssetCache::open(&store).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    std::fs::write(store.join("daemon.json"), serde_json::to_vec(&serde_json::json!({
        "pid": std::process::id(), "endpoint": listener.local_addr().unwrap().to_string(), "token": "unresponsive"
    })).unwrap()).unwrap();
    let started = std::time::Instant::now();
    let result = lab(&store, root, &["images", "--json"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(store.join("daemon.json").exists());
}
