mod support;

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn lab(store: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lab"))
        .arg("--cache-dir")
        .arg(store)
        .args(arguments)
        .output()
        .unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn import(store: &Path, root: &Path, runnable: bool) -> String {
    support::create_asset(root);
    if runnable {
        let manifest = std::fs::read_to_string(root.join("asset.yaml"))
            .unwrap()
            .replace(
                "  type: robot",
                "  type: robot\n  runtime:\n    engine: kinematic\n    config: world.yaml",
            );
        std::fs::write(root.join("asset.yaml"), manifest).unwrap();
        std::fs::write(
            root.join("world.yaml"),
            "engine: kinematic\nworld_id: test-world\n",
        )
        .unwrap();
    }
    let layout = root.with_extension("layout");
    let packed = Command::new(env!("CARGO_BIN_EXE_lab"))
        .arg("pack")
        .arg(root)
        .arg("--output")
        .arg(&layout)
        .output()
        .unwrap();
    assert!(
        packed.status.success(),
        "{}",
        String::from_utf8_lossy(&packed.stderr)
    );
    let imported = lab(
        store,
        &[
            "asset",
            "import",
            layout.to_str().unwrap(),
            "--tag",
            "localhost/lab/test:dev",
        ],
    );
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    String::from_utf8(imported.stdout)
        .unwrap()
        .trim()
        .to_owned()
}
struct StopGuard {
    store: PathBuf,
    names: Vec<String>,
}
impl Drop for StopGuard {
    fn drop(&mut self) {
        for name in &self.names {
            let _ = lab(&self.store, &["instance", "stop", name]);
        }
    }
}

#[test]
fn instances_have_independent_lifecycles_and_keep_their_original_package() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let digest = import(&store, &temp.path().join("asset"), true);
    let _cleanup = StopGuard {
        store: store.clone(),
        names: vec!["one".into(), "two".into()],
    };
    let created = success(lab(
        &store,
        &[
            "instance",
            "create",
            "localhost/lab/test:dev",
            "--name",
            "one",
            "--json",
        ],
    ));
    assert_eq!(created["state"], "created");
    assert_eq!(created["manifest_digest"], digest);
    assert!(
        !lab(&store, &["instance", "create", &digest, "--name", "one"])
            .status
            .success()
    );
    let first = success(lab(&store, &["instance", "start", "one", "--json"]));
    assert_eq!(first["state"], "running");
    let second = success(lab(
        &store,
        &["instance", "run", &digest, "--name", "two", "--json"],
    ));
    assert_eq!(second["state"], "running");
    assert_ne!(first["endpoint"], second["endpoint"]);
    assert_ne!(first["pid"], second["pid"]);
    let devices = success(lab(&store, &["instance", "devices", "one", "--json"]));
    assert_eq!(devices.as_array().unwrap().len(), 4);
    let action = success(lab(
        &store,
        &[
            "instance",
            "send",
            "one",
            "sim:test-world:heater-1",
            "start",
            "--json",
        ],
    ));
    assert_eq!(action["status"], "PENDING");
    assert!(!lab(&store, &["instance", "remove", "one"]).status.success());
    let stopped = success(lab(&store, &["instance", "stop", "one", "--json"]));
    assert_eq!(stopped["state"], "exited");
    assert_eq!(
        success(lab(&store, &["instance", "stop", "one", "--json"]))["state"],
        "exited"
    );
    let removed_tag = lab(&store, &["asset", "remove", "localhost/lab/test:dev"]);
    assert!(removed_tag.status.success());
    assert!(!lab(&store, &["asset", "remove", &digest]).status.success());
    assert!(
        success(lab(&store, &["asset", "prune", "--json"]))["packages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let restarted = success(lab(&store, &["instance", "start", "one", "--json"]));
    assert_eq!(restarted["manifest_digest"], digest);
    assert!(restarted["generation"].as_u64().unwrap() > first["generation"].as_u64().unwrap());
    assert!(lab(&store, &["instance", "logs", "one", "--tail", "10"])
        .status
        .success());
    let json = success(lab(&store, &["instance", "inspect", "one", "--json"]));
    assert!(json.get("token").is_none());
    assert!(json.get("auth_token").is_none());
    for name in ["one", "two"] {
        assert!(lab(&store, &["instance", "stop", name]).status.success());
        assert!(lab(&store, &["instance", "remove", name]).status.success());
    }
    assert_eq!(
        success(lab(&store, &["instance", "list", "--all", "--json"])),
        serde_json::json!([])
    );
    assert_eq!(
        success(lab(&store, &["asset", "prune", "--json"]))["packages"],
        serde_json::json!([digest])
    );
}

#[test]
fn visual_assets_and_invalid_names_cannot_create_instances() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let digest = import(&store, &temp.path().join("visual"), false);
    let rejected = lab(&store, &["instance", "create", &digest, "--name", "visual"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("runtime"));
    for name in ["../escape", ".", "..", "UPPER SPACE"] {
        assert!(
            !lab(&store, &["instance", "create", &digest, "--name", name])
                .status
                .success()
        );
    }
    assert_eq!(
        success(lab(&store, &["instance", "list", "--all", "--json"])),
        serde_json::json!([])
    );
}

#[test]
fn lifecycle_rejects_invalid_names_before_opening_any_lock_file() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    success(lab(&store, &["instance", "list", "--all", "--json"]));
    let outside = store.join("escape");
    std::fs::create_dir(&outside).unwrap();
    for command in ["inspect", "remove", "stop", "start"] {
        assert!(!lab(&store, &["instance", command, "../escape"])
            .status
            .success());
    }
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
    assert!(!store.join("escape.lock").exists());
}

#[test]
fn invalid_world_configuration_is_retained_as_a_removable_failed_instance() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let root = temp.path().join("asset");
    import(&store, &root, true);
    std::fs::write(
        root.join("world.yaml"),
        "engine: kinematic\nentrypoint: execute-anything\n",
    )
    .unwrap();
    let layout = temp.path().join("invalid.layout");
    let packed = Command::new(env!("CARGO_BIN_EXE_lab"))
        .arg("pack")
        .arg(&root)
        .arg("--output")
        .arg(&layout)
        .output()
        .unwrap();
    assert!(packed.status.success());
    let imported = lab(&store, &["asset", "import", layout.to_str().unwrap()]);
    assert!(imported.status.success());
    let digest = String::from_utf8(imported.stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert!(
        !lab(&store, &["instance", "create", &digest, "--name", "bad"])
            .status
            .success()
    );
    assert_eq!(
        success(lab(&store, &["instance", "inspect", "bad", "--json"]))["state"],
        "failed"
    );
    assert!(!lab(&store, &["asset", "remove", &digest]).status.success());
    assert!(lab(&store, &["instance", "remove", "bad"]).status.success());
    assert!(lab(&store, &["asset", "remove", &digest]).status.success());
}

#[test]
fn retagging_does_not_change_an_existing_instances_world() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let root = temp.path().join("asset");
    let original = import(&store, &root, true);
    success(lab(
        &store,
        &[
            "instance",
            "create",
            "localhost/lab/test:dev",
            "--name",
            "fixed",
            "--json",
        ],
    ));
    let _cleanup = StopGuard {
        store: store.clone(),
        names: vec!["fixed".into()],
    };
    std::fs::write(
        root.join("world.yaml"),
        "world_id: different-world\nengine: kinematic\n",
    )
    .unwrap();
    let new_layout = temp.path().join("retagged.layout");
    let pack = Command::new(env!("CARGO_BIN_EXE_lab"))
        .arg("pack")
        .arg(&root)
        .arg("--output")
        .arg(&new_layout)
        .output()
        .unwrap();
    assert!(pack.status.success());
    let imported = lab(
        &store,
        &[
            "asset",
            "import",
            new_layout.to_str().unwrap(),
            "--tag",
            "localhost/lab/test:dev",
        ],
    );
    assert!(imported.status.success());
    let running = success(lab(&store, &["instance", "start", "fixed", "--json"]));
    assert_eq!(running["manifest_digest"], original);
    let devices = success(lab(&store, &["instance", "devices", "fixed", "--json"]));
    assert_eq!(
        devices[0]["device_id"].as_str().unwrap().split(':').nth(1),
        Some("test-world")
    );
    assert!(lab(&store, &["instance", "stop", "fixed"]).status.success());
    assert!(lab(&store, &["instance", "remove", "fixed"])
        .status
        .success());
}

#[cfg(unix)]
#[test]
fn a_crashed_worker_is_detected_and_the_instance_can_restart() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let digest = import(&store, &temp.path().join("asset"), true);
    let _cleanup = StopGuard {
        store: store.clone(),
        names: vec!["crash".into()],
    };
    let running = success(lab(
        &store,
        &["instance", "run", &digest, "--name", "crash", "--json"],
    ));
    let own_pid = running["pid"].as_u64().unwrap().to_string();
    assert!(Command::new("kill")
        .args(["-KILL", &own_pid])
        .status()
        .unwrap()
        .success());
    std::thread::sleep(std::time::Duration::from_millis(80));
    assert_eq!(
        success(lab(&store, &["instance", "inspect", "crash", "--json"]))["state"],
        "failed"
    );
    assert!(!lab(&store, &["asset", "remove", &digest]).status.success());
    assert_eq!(
        success(lab(&store, &["instance", "start", "crash", "--json"]))["state"],
        "running"
    );
    assert!(lab(&store, &["instance", "stop", "crash"]).status.success());
    assert!(lab(&store, &["instance", "remove", "crash"])
        .status
        .success());
}

#[cfg(unix)]
#[test]
fn startup_refuses_a_log_symlink_without_touching_the_target() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let digest = import(&store, &temp.path().join("asset"), true);
    success(lab(
        &store,
        &["instance", "create", &digest, "--name", "linked", "--json"],
    ));
    let outside = temp.path().join("keep.log");
    std::fs::write(&outside, b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, store.join("instances/linked/instance.log")).unwrap();
    let _cleanup = StopGuard {
        store: store.clone(),
        names: vec!["linked".into()],
    };
    assert!(!lab(&store, &["instance", "start", "linked"])
        .status
        .success());
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn a_dangling_instance_directory_does_not_reserve_a_name_or_package() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let digest = import(&store, &temp.path().join("asset"), true);
    success(lab(&store, &["instance", "list", "--all", "--json"]));
    std::os::unix::fs::symlink(temp.path().join("absent"), store.join("instances/dangling"))
        .unwrap();
    assert!(!lab(
        &store,
        &["instance", "create", &digest, "--name", "dangling"]
    )
    .status
    .success());
    assert_eq!(
        success(lab(&store, &["instance", "list", "--all", "--json"])),
        serde_json::json!([])
    );
    assert!(lab(&store, &["asset", "remove", "localhost/lab/test:dev"])
        .status
        .success());
    assert!(lab(&store, &["asset", "remove", &digest]).status.success());
}
