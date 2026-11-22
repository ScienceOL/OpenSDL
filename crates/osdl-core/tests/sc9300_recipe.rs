use std::time::Duration;

use osdl_core::{protocol::DeviceCommand, OsdlConfig, OsdlEngine};
use serde_json::json;

#[tokio::test]
async fn configured_sc9300_exposes_closed_reads_and_stays_offline_without_identity() {
    let port = if cfg!(windows) {
        "COM65534"
    } else {
        "/dev/opensdl-sc9300-test-no-such-port"
    };
    let config: OsdlConfig = serde_yaml::from_str(&format!(
        "sc9300_instruments:\n  - id: test-workstation\n    port: {port}\n"
    ))
    .unwrap();
    let mut engine = OsdlEngine::new(config, vec![]);
    let handle = engine.handle();
    let task = tokio::spawn(async move { engine.run().await });

    let device = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(device) = handle.get_device("test-workstation").await {
                break device;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the configured workstation should be listed");
    assert!(!device.online);
    assert_eq!(device.device_type, "prcxi.sc9300");
    assert_eq!(
        device
            .actions
            .iter()
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>(),
        ["device_info", "status"]
    );

    for (action, params, message) in [
        ("move", json!({}), "unsupported SC9300 read"),
        (
            "status",
            json!({"command": 0x0101}),
            "SC9300 reads do not accept arguments",
        ),
        ("device_info", json!({}), "SC9300 connection unavailable"),
    ] {
        let command = DeviceCommand {
            command_id: action.into(),
            device_id: device.id.clone(),
            action: action.into(),
            params,
        };
        assert_eq!(handle.send_command(command).await.unwrap_err(), message);
    }

    handle.request_stop();
    task.await.unwrap();
}
