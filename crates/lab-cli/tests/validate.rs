use std::{path::Path, process::Command};

#[test]
fn validate_json_reports_real_dynamic_execution() {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/instruments/analytical-balance/package");
    let output = Command::new(env!("CARGO_BIN_EXE_lab"))
        .args(["validate", "--json"])
        .arg(package)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["name"], "analytical-balance");
    assert_eq!(report["dynamic"]["engine"], "kinematic");
    assert_eq!(report["dynamic"]["actions_executed"], 7);
    assert!(report["dynamic"]["observations_checked"].as_u64().unwrap() >= 7);
}
