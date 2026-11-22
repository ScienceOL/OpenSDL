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
                .env_remove("OSDL_LOCAL_CAPABILITY")
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

#[tokio::test]
async fn serve_displays_the_cli_name_ahead_of_the_environment_default() {
    assert_profile_server(false).await;
}

#[tokio::test]
async fn provisioned_local_authority_can_edit_the_profile_without_discovery() {
    assert_profile_server(true).await;
}

async fn assert_profile_server(writable: bool) {
    let paths = osdl_server::Paths::discover().unwrap();
    let instance = format!("serve-profile-{writable}-{}", std::process::id());
    let data = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_lab"))
        .args(["serve", "--simulation", "--auth", "static"])
        .args(["--instance", &instance, "--socket", "disabled"])
        .args(["--listen", "127.0.0.1:0", "--data-dir"])
        .arg(data.path())
        .args(["--name", "Microscopy"])
        .env("OSDL_LOCAL_CAPABILITY", writable.to_string())
        .env("OSDL_LAB_NAME", "Different name")
        .env("OSDL_AUTH_TOKEN", "profile-test")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let address = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("lab serve exited early: {status}");
            }
            if let Some(address) = osdl_server::lockfile::find(&paths.lock_dir(), &instance)
                .and_then(|record| record.listen_addr)
            {
                break address;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    let endpoint = format!("http://{address}");
    let status = tokio::process::Command::new(env!("CARGO_BIN_EXE_lab"))
        .args(["status", "--endpoint", &endpoint])
        .env("OSDL_AUTH_TOKEN", "profile-test")
        .output()
        .await
        .unwrap();
    assert!(status.status.success(), "{:?}", status.stderr);
    let output = String::from_utf8(status.stdout).unwrap();
    assert!(
        output.lines().any(|line| line == "lab:         Microscopy"),
        "{output}"
    );

    use osdl_proto::v1 as pb;
    let channel = tonic::transport::Endpoint::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client =
        pb::osdl_client::OsdlClient::with_interceptor(channel, |mut req: tonic::Request<()>| {
            req.metadata_mut()
                .insert("authorization", "Bearer profile-test".parse().unwrap());
            Ok(req)
        });
    let profile = client
        .status(pb::StatusRequest {})
        .await
        .unwrap()
        .into_inner()
        .laboratory
        .unwrap();
    assert_eq!(profile.writable, writable);
    let result = client
        .update_lab_profile(pb::UpdateLabProfileRequest {
            name: "Updated microscopy lab".into(),
            description: String::new(),
            revision: profile.revision,
        })
        .await;
    if writable {
        assert_eq!(result.unwrap().into_inner().name, "Updated microscopy lab");
        assert!(!paths
            .lock_dir()
            .join(format!("private-{instance}"))
            .exists());
    } else {
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }
    client.shutdown(pb::ShutdownRequest {}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(osdl_server::lockfile::find(&paths.lock_dir(), &instance).is_none());
}
