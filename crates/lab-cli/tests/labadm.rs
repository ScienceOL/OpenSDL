use std::process::Command;

#[test]
fn topology_and_device_commands_are_available() {
    let labadm = env!("CARGO_BIN_EXE_labadm");
    let help = Command::new(labadm).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for command in [
        "init", "serve", "join", "admit", "forget", "devices", "send", "events",
    ] {
        assert!(help.contains(command), "missing labadm command {command}");
    }
}
