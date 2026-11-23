use std::time::Duration;

use osdl_core::{protocol::DeviceCommand, OsdlConfig, OsdlEngine};
use serde_json::json;

#[tokio::test]
async fn configured_sc9300_exposes_closed_actions_and_stays_offline_without_identity() {
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
        [
            "device_info",
            "status",
            "position",
            "telemetry",
            "check_limit",
            "wait_idle",
            "stop",
            "reset",
            "clear_error",
            "set_motors",
            "move",
            "jog",
            "jaw",
            "pick_up_tips",
            "set_back_tips",
            "aspirate",
            "dispense",
            "homogenize",
            "clamp_plate",
            "set_temperature",
            "oscillate",
            "temperature_and_oscillate",
            "magnetic_stand",
            "set_light",
            "set_ultraviolet",
            "set_filter_fan",
        ]
    );

    for (action, params, message) in [
        ("write_firmware", json!({}), "unsupported SC9300 action"),
        (
            "status",
            json!({"command": 0x0101}),
            "unexpected argument 'command'",
        ),
        (
            "move",
            json!({"axis":"left","x":0,"y":0}),
            "missing argument 'z'",
        ),
        (
            "set_motors",
            json!({"enabled":1}),
            "invalid argument 'enabled'",
        ),
        (
            "pick_up_tips",
            json!({"axis":"gripper","x":0,"y":0,"zSeat":0}),
            "invalid argument 'axis'",
        ),
        (
            "jog",
            json!({"direction":"x_left","distanceMm":201}),
            "invalid argument 'distanceMm'",
        ),
        ("device_info", json!({}), "SC9300 connection unavailable"),
        (
            "move",
            json!({"axis":"left","x":0,"y":0,"z":0}),
            "SC9300 connection unavailable",
        ),
    ] {
        let command = DeviceCommand {
            command_id: action.into(),
            device_id: device.id.clone(),
            action: action.into(),
            params,
        };
        let error = handle.send_command(command).await.unwrap_err();
        assert!(error.contains(message), "{action} returned {error}");
    }

    handle.request_stop();
    task.await.unwrap();
}
