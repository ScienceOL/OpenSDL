mod support;

use std::process::Command;
use support::create_asset;

#[test]
fn validate_pack_and_inspect_form_a_local_asset_workflow() {
    let temp = tempfile::tempdir().expect("create CLI test root");
    let asset = temp.path().join("asset");
    create_asset(&asset);
    let layout = temp.path().join("layout");

    let validate = lab()
        .arg("validate")
        .arg(&asset)
        .output()
        .expect("validate");
    assert!(validate.status.success(), "{}", stderr(&validate));
    assert!(String::from_utf8_lossy(&validate.stdout).contains("ur5e 1.0.0"));

    let pack = lab()
        .arg("pack")
        .arg(&asset)
        .arg("--output")
        .arg(&layout)
        .output()
        .expect("pack");
    assert!(pack.status.success(), "{}", stderr(&pack));
    assert!(String::from_utf8_lossy(&pack.stdout).starts_with("sha256:"));

    let inspect = lab()
        .arg("inspect")
        .arg(&layout)
        .arg("--json")
        .output()
        .expect("inspect");
    assert!(inspect.status.success(), "{}", stderr(&inspect));
    let document: serde_json::Value =
        serde_json::from_slice(&inspect.stdout).expect("inspect JSON");
    assert_eq!(
        document["artifactType"],
        "application/vnd.opensdl.virtual-asset.v1"
    );
    assert!(document["manifestDigest"]
        .as_str()
        .expect("manifest digest")
        .starts_with("sha256:"));
}

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lab"))
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn local_inventory_tags_and_prune_manage_complete_packages() {
    let temp = tempfile::tempdir().expect("create inventory test root");
    let asset = temp.path().join("asset");
    create_asset(&asset);
    let layout = temp.path().join("layout");
    let packed = lab()
        .args(["pack"])
        .arg(&asset)
        .arg("--output")
        .arg(&layout)
        .output()
        .unwrap();
    assert!(packed.status.success(), "{}", stderr(&packed));
    let digest = String::from_utf8(packed.stdout).unwrap().trim().to_owned();
    let store = temp.path().join("store");
    let invoke = |args: &[&str]| {
        lab()
            .arg("--cache-dir")
            .arg(&store)
            .args(args)
            .output()
            .unwrap()
    };
    let empty = invoke(&["asset", "list", "--json"]);
    assert!(empty.status.success(), "{}", stderr(&empty));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&empty.stdout).unwrap(),
        serde_json::json!([])
    );
    let imported = invoke(&[
        "asset",
        "import",
        layout.to_str().unwrap(),
        "--tag",
        "localhost/library/ur5e:1.0.0",
    ]);
    assert!(imported.status.success(), "{}", stderr(&imported));
    let listing = invoke(&["asset", "list", "--json"]);
    assert!(listing.status.success(), "{}", stderr(&listing));
    let rows: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["name"], "ur5e");
    assert_eq!(rows[0]["manifest_digest"], digest);
    assert_eq!(
        rows[0]["references"],
        serde_json::json!(["oci://localhost/library/ur5e:1.0.0"])
    );
    let tagged = invoke(&[
        "asset",
        "tag",
        "localhost/library/ur5e:1.0.0",
        "localhost/library/ur5e:dev",
    ]);
    assert!(tagged.status.success(), "{}", stderr(&tagged));
    for reference in ["localhost/library/ur5e:1.0.0", "localhost/library/ur5e:dev"] {
        let removed = invoke(&["asset", "remove", reference]);
        assert!(removed.status.success(), "{}", stderr(&removed));
    }
    let dry_run = invoke(&["asset", "prune", "--dry-run", "--json"]);
    assert!(dry_run.status.success(), "{}", stderr(&dry_run));
    let report: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(report["packages"], serde_json::json!([digest]));
    let inspect = invoke(&["asset", "inspect", &digest, "--json"]);
    assert!(inspect.status.success(), "{}", stderr(&inspect));
    let prune = invoke(&["asset", "prune", "--json"]);
    assert!(prune.status.success(), "{}", stderr(&prune));
    assert!(!invoke(&["asset", "inspect", &digest, "--json"])
        .status
        .success());
    let final_list = invoke(&["asset", "list", "--json"]);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&final_list.stdout).unwrap(),
        serde_json::json!([])
    );
}

#[test]
fn inventory_refuses_corrupt_package_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let asset = temp.path().join("asset");
    create_asset(&asset);
    let layout = temp.path().join("layout");
    let pack = lab()
        .arg("pack")
        .arg(&asset)
        .arg("--output")
        .arg(&layout)
        .output()
        .unwrap();
    assert!(pack.status.success(), "{}", stderr(&pack));
    let digest = String::from_utf8(pack.stdout).unwrap().trim().to_owned();
    let store = temp.path().join("store");
    let import = lab()
        .arg("--cache-dir")
        .arg(&store)
        .args(["asset", "import"])
        .arg(&layout)
        .output()
        .unwrap();
    assert!(import.status.success(), "{}", stderr(&import));
    std::fs::write(
        store
            .join("blobs/sha256")
            .join(digest.strip_prefix("sha256:").unwrap()),
        b"corrupt",
    )
    .unwrap();
    let inspect = lab()
        .arg("--cache-dir")
        .arg(&store)
        .args(["asset", "inspect", &digest, "--json"])
        .output()
        .unwrap();
    assert!(!inspect.status.success());
}

#[test]
fn an_invalid_import_tag_leaves_the_inventory_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let asset = temp.path().join("asset");
    create_asset(&asset);
    let layout = temp.path().join("layout");
    let pack = lab()
        .arg("pack")
        .arg(&asset)
        .arg("--output")
        .arg(&layout)
        .output()
        .unwrap();
    assert!(pack.status.success());
    let store = temp.path().join("store");
    let failed = lab().arg("--cache-dir").arg(&store).args(["asset", "import"]).arg(&layout).args(["--tag", "localhost/lab/ur5e@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]).output().unwrap();
    assert!(!failed.status.success());
    let listing = lab()
        .arg("--cache-dir")
        .arg(&store)
        .args(["asset", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&listing.stdout).unwrap(),
        serde_json::json!([])
    );
}
