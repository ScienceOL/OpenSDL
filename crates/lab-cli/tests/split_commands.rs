mod support;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn split(root: &Path, output: &Path, parts: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lab"));
    command
        .arg("split")
        .arg(root)
        .arg("--output")
        .arg(output)
        .arg("--json");
    for part in parts {
        command.arg("--part").arg(part);
    }
    command.output().unwrap()
}
fn fixture(root: &Path) {
    support::create_asset(root);
    fs::create_dir(root.join("textures")).unwrap();
    fs::write(root.join("textures/color.png"), b"texture bytes").unwrap();
    fs::write(
        root.join("scene.usda"),
        r#"#usda 1.0
(defaultPrim = "Robot")
def Xform "Robot" {
    def Xform "Arm" {
        custom asset texture = @textures/color.png@
    }
    def Xform "Body" {}
}
def Xform "Other" {
    def Xform "Arm" {}
}
"#,
    )
    .unwrap();
}
#[test]
fn split_preserves_portable_dependencies_and_deterministic_lineage() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("robot");
    fixture(&source);
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for target in [&first, &second] {
        let result = split(&source, target, &["arm=/Robot/Arm", "body=/Robot/Body"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["children"].as_array().unwrap().len(), 2);
    }
    fs::remove_dir_all(source).unwrap();
    let child = first.join("arm");
    let text = fs::read_to_string(child.join("source/arm.usda")).unwrap();
    assert!(text.contains("dependencies/textures/color.png"), "{text}");
    assert!(!text.contains("Body"));
    assert!(!text.contains("Other"));
    assert_eq!(
        fs::read(child.join("source/dependencies/textures/color.png")).unwrap(),
        b"texture bytes"
    );
    let manifest = opensdl_assets::VirtualAssetManifest::load(&child)
        .unwrap()
        .manifest;
    assert_eq!(manifest.metadata.lineage.unwrap().source_path, "/Robot/Arm");
    let first_image = opensdl_assets::pack_directory(&child, &temp.path().join("image-a")).unwrap();
    let second_image =
        opensdl_assets::pack_directory(&second.join("arm"), &temp.path().join("image-b")).unwrap();
    assert_eq!(first_image.manifest_digest, second_image.manifest_digest);
}
#[test]
fn nonexistent_full_path_and_invalid_names_never_publish_partial_children() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("robot");
    fixture(&source);
    let output = temp.path().join("parts");
    let result = split(&source, &output, &["body=/Robot/Body", "arm=/Missing/Arm"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("/Missing/Arm"));
    assert!(!output.exists());
    for name in ["..", ".", "Upper", "has_underscore", "-prefix"] {
        let part = format!("{name}=/Robot/Arm");
        assert!(!split(&source, &output, &[&part]).status.success());
        assert!(!output.exists());
    }
    assert!(split(&source, &output, &["arm=/Robot/Arm"])
        .status
        .success());
}
#[test]
fn missing_dependency_leaves_output_retryable() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("robot");
    fixture(&source);
    fs::remove_file(source.join("textures/color.png")).unwrap();
    let output = temp.path().join("parts");
    let result = split(&source, &output, &["arm=/Robot/Arm"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("non-portable USD dependency"));
    assert!(!output.exists());
}
