use std::{process::Stdio, time::Duration};

/// A listener that cannot bind must end `lab serve` with an error so its
/// supervisor can retry, rather than leave a process with no listener.
#[test]
fn serve_exits_when_its_listener_cannot_bind() {
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = occupied.local_addr().unwrap().to_string();
    let data = tempfile::tempdir().unwrap();
    let output = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_lab"))
                .args(["serve", "--simulation", "--auth", "static"])
                .args(["--instance", "listener-failure", "--socket", "disabled"])
                .args(["--listen", &address, "--data-dir"])
                .arg(data.path())
                .env("OSDL_AUTH_TOKEN", "listener-failure")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
                .await
                .expect("lab serve exits when its listener cannot bind")
                .unwrap()
        });
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("lab: serve: io error"), "{stderr}");
}
