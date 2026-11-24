use std::process::Command;

#[test]
fn help_identifies_lab_as_the_user_facing_cli() {
    let output = Command::new(env!("CARGO_BIN_EXE_lab"))
        .arg("--help")
        .output()
        .expect("run lab --help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output is UTF-8");
    assert!(stdout.contains("OpenSDL command-line interface"));
    assert!(stdout.contains("Usage: lab"));
    assert!(stdout.contains("validate"));
    assert!(stdout.contains("pack"));
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("push"));
    assert!(stdout.contains("pull"));
    assert!(stdout.contains("daemon"));
    assert!(stdout.contains("build"));
    assert!(stdout.contains("split"));
}

#[test]
fn account_membership_mutation_requires_the_explicit_local_instance() {
    let lab = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lab"));
        for option in ["OSDL_INSTANCE", "OSDL_ENDPOINT", "OSDL_AUTH_TOKEN"] {
            command.env_remove(option);
        }
        command.args(["auth", "account", "grant", "member", "--role", "viewer"]);
        command
    };
    let missing = lab().output().expect("run account grant");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("requires --instance"));

    let remote = lab()
        .args([
            "--instance",
            "lab",
            "--endpoint",
            "http://198.51.100.1:50051",
        ])
        .output()
        .expect("run account grant with endpoint");
    assert!(!remote.status.success());
    assert!(String::from_utf8_lossy(&remote.stderr).contains("on the laboratory host"));
}
