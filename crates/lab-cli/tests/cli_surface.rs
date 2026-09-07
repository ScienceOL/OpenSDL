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
}
