use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::config::{AdapterConfig, EspNowGatewayConfig, MqttConfig, OsdlConfig};
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::{
    DeviceCommand, EmbeddedBroker, EventStore, MdnsAdvertiser, OsdlEngine, OsdlEvent,
};

#[tokio::main]
async fn main() {
    env_logger::init();

    // ESP-NOW gateway is opt-in via env var so a host without the board
    // plugged in still boots cleanly. Example:
    //   OSDL_ESPNOW_PORT=/dev/cu.usbserial-A5069RR4 cargo run -p osdl-cli
    let espnow_gateways = match std::env::var("OSDL_ESPNOW_PORT") {
        Ok(port) if !port.is_empty() => vec![EspNowGatewayConfig {
            port,
            baud_rate: std::env::var("OSDL_ESPNOW_BAUD")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(115200),
        }],
        _ => vec![],
    };

    // TODO: load config from file / CLI args
    let config = OsdlConfig {
        mqtt: Some(MqttConfig::default()),
        adapters: vec![AdapterConfig {
            adapter_type: "unilabos".into(),
            registry_path: Some("registry/unilabos".into()),
        }],
        espnow_gateways,
        buses: vec![],
    };

    // Start embedded MQTT broker + mDNS only when MQTT is enabled.
    let _broker = config
        .mqtt
        .as_ref()
        .map(|c| EmbeddedBroker::start(c.port).expect("Failed to start MQTT broker"));
    let _mdns = config
        .mqtt
        .as_ref()
        .map(|c| MdnsAdvertiser::start(c.port).expect("Failed to start mDNS"));

    // Give broker a moment to bind the port
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Open event store (SQLite)
    let store = EventStore::open("osdl.db").expect("Failed to open event store");

    let adapters: Vec<Box<dyn osdl_core::adapter::ProtocolAdapter>> =
        vec![Box::new(UniLabOsAdapter::new(DriverRegistry::with_builtins()))];

    let mut engine = OsdlEngine::new(config, adapters).with_store(store);

    // Event consumer + optional auto-drive.
    //
    // `OSDL_DEMO_DEVICE_TYPE` turns on a one-shot demo: when the first Device
    // with that `device_type` comes online, inject a small canned command
    // sequence (by default: initialize → query_position → query_valve_position
    // for a Runze pump). Lets us verify the full Mac → gateway → child →
    // RS-485 → device control loop without a separate example binary.
    //
    // Examples:
    //   OSDL_DEMO_DEVICE_TYPE=syringe_pump_with_valve.runze.SY03B-T06 \
    //       OSDL_ESPNOW_PORT=/dev/cu.usbserial-XXX cargo run -p osdl-cli
    let event_rx = engine.take_event_rx();
    let cmd_tx = engine.command_sender();
    let demo_device_type = std::env::var("OSDL_DEMO_DEVICE_TYPE").ok();
    tokio::spawn(async move {
        let mut rx = event_rx.lock().await.take().unwrap();
        let mut demo_triggered = false;
        while let Some(event) = rx.recv().await {
            log::info!("Event: {:?}", event);
            if let (false, Some(target), OsdlEvent::DeviceOnline(device)) =
                (demo_triggered, demo_device_type.as_deref(), &event)
            {
                if device.device_type == target {
                    demo_triggered = true;
                    let device_id = device.id.clone();
                    let tx = cmd_tx.clone();
                    log::info!(
                        "demo: driving {} with canned command sequence",
                        device_id
                    );
                    tokio::spawn(async move {
                        let script = [
                            ("cmd-init",   "initialize",           serde_json::json!({})),
                            ("cmd-qpos",   "query_position",       serde_json::json!({})),
                            ("cmd-qvalve", "query_valve_position", serde_json::json!({})),
                        ];
                        // Give transport + device a beat to settle after online.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        for (id, action, params) in script {
                            let _ = tx.send(DeviceCommand {
                                command_id: id.into(),
                                device_id: device_id.clone(),
                                action: action.into(),
                                params,
                            });
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                    });
                }
            }
        }
    });

    log::info!("Starting OpenSDL...");
    engine.run().await;
}
