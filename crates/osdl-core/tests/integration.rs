use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;

#[test]
fn test_load_registry() {
    let mut adapter = UniLabOsAdapter::new();
    let result = adapter.load_registry("../../registry/unilabos");
    assert!(result.is_ok(), "Failed to load registry: {:?}", result);

    let matched = adapter.match_hardware("heater_stirrer_dalong");
    assert!(matched.is_some(), "Should match heater_stirrer_dalong");

    let m = matched.unwrap();
    assert_eq!(m.device_type, "heater_stirrer_dalong");
    assert!(!m.actions.is_empty(), "Should have actions");

    let action_names: Vec<&str> = m.actions.iter().map(|a| a.name.as_str()).collect();
    assert!(action_names.contains(&"set_temperature"));
    assert!(action_names.contains(&"set_stir_speed"));
    assert!(action_names.contains(&"stop"));
}

#[test]
fn test_unknown_hardware() {
    let mut adapter = UniLabOsAdapter::new();
    adapter.load_registry("../../registry/unilabos").unwrap();

    let matched = adapter.match_hardware("nonexistent_device_xyz");
    assert!(matched.is_none(), "Should not match unknown hardware");
}

#[test]
fn test_engine_creation() {
    use osdl_core::config::{AdapterConfig, MqttConfig, OsdlConfig};
    use osdl_core::{EventStore, OsdlEngine, OsdlStatus};

    let config = OsdlConfig {
        mqtt: MqttConfig::default(),
        adapters: vec![AdapterConfig {
            adapter_type: "unilabos".into(),
            registry_path: Some("../../registry/unilabos".into()),
        }],
    };

    let store = EventStore::in_memory().unwrap();
    let adapters: Vec<Box<dyn ProtocolAdapter>> = vec![Box::new(UniLabOsAdapter::new())];
    let engine = OsdlEngine::new(config, adapters).with_store(store);

    assert_eq!(engine.status(), OsdlStatus::Disconnected);
    let _rx = engine.take_event_rx();
}

#[test]
fn test_event_store_logging() {
    use osdl_core::event::OsdlEvent;
    use osdl_core::protocol::{DeviceCommand, DeviceStatus};
    use osdl_core::EventStore;
    use std::collections::HashMap;

    let store = EventStore::in_memory().unwrap();

    // Log an event
    let status = DeviceStatus {
        device_id: "node-01:heater".into(),
        timestamp: 1000,
        properties: {
            let mut m = HashMap::new();
            m.insert("temperature".into(), serde_json::json!(25.3));
            m
        },
    };
    store.log_event(&OsdlEvent::DeviceStatus(status));

    // Log a command
    let cmd = DeviceCommand {
        command_id: "cmd-001".into(),
        device_id: "node-01:heater".into(),
        action: "set_temperature".into(),
        params: serde_json::json!({"temperature": 80}),
    };
    store.log_command(&cmd);

    // Log serial bytes
    store.log_serial("node-01", "tx", &[0xFE, 0xB1, 0x01, 0x50]);
    store.log_serial("node-01", "rx", &[0xFE, 0xB2, 0x00, 0x19]);

    // Query events
    let events = store.query_events(Some("device_status"), None, None, 10);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "device_status");
    assert!(events[0].payload.contains("temperature"));

    // Query serial log
    let serial = store.query_serial("node-01", None, 10);
    assert_eq!(serial.len(), 2);
    assert_eq!(serial[0].direction, "tx");
    assert_eq!(serial[0].bytes, vec![0xFE, 0xB1, 0x01, 0x50]);
    assert_eq!(serial[1].direction, "rx");
}

#[test]
fn test_event_store_query_filters() {
    use osdl_core::event::OsdlEvent;
    use osdl_core::protocol::Device;
    use osdl_core::EventStore;

    let store = EventStore::in_memory().unwrap();

    // Log multiple event types
    let device = Device {
        id: "node-01:heater".into(),
        node_id: "node-01".into(),
        device_type: "heater_stirrer_dalong".into(),
        adapter: "unilabos".into(),
        description: "test".into(),
        online: true,
        properties: Default::default(),
        actions: vec![],
    };
    store.log_event(&OsdlEvent::DeviceOnline(device));
    store.log_event(&OsdlEvent::DeviceOffline {
        device_id: "node-01:heater".into(),
    });
    store.log_event(&OsdlEvent::UnknownNode {
        node_id: "node-99".into(),
        hardware_id: "mystery".into(),
    });

    // All events
    let all = store.query_events(None, None, None, 100);
    assert_eq!(all.len(), 3);

    // Filter by type
    let online = store.query_events(Some("device_online"), None, None, 100);
    assert_eq!(online.len(), 1);

    let unknown = store.query_events(Some("unknown_node"), None, None, 100);
    assert_eq!(unknown.len(), 1);
    assert!(unknown[0].payload.contains("mystery"));
}

#[test]
fn test_runze_pump_via_adapter() {
    use osdl_core::adapter::ProtocolAdapter;
    use osdl_core::adapter::unilabos::UniLabOsAdapter;
    use osdl_core::protocol::DeviceCommand;

    let mut adapter = UniLabOsAdapter::new();
    adapter.load_registry("../../registry/unilabos").unwrap();

    // Match the T06 pump
    let matched = adapter.match_hardware("syringe_pump_with_valve.runze.SY03B-T06");
    assert!(matched.is_some(), "Should match Runze SY03B-T06");
    let m = matched.unwrap();
    assert!(m.actions.iter().any(|a| a.name == "initialize"));
    assert!(m.actions.iter().any(|a| a.name == "set_position"));
    assert!(m.actions.iter().any(|a| a.name == "set_valve_position"));

    // Encode initialize command
    let cmd = DeviceCommand {
        command_id: "cmd-001".into(),
        device_id: "pump-01".into(),
        action: "initialize".into(),
        params: serde_json::json!({}),
    };
    let bytes = adapter
        .encode_command("syringe_pump_with_valve.runze.SY03B-T06", &cmd)
        .unwrap();
    assert_eq!(bytes, b"/1ZR\r\n");

    // Encode set_position
    let cmd = DeviceCommand {
        command_id: "cmd-002".into(),
        device_id: "pump-01".into(),
        action: "set_position".into(),
        params: serde_json::json!({"position": 12.5}),
    };
    let bytes = adapter
        .encode_command("syringe_pump_with_valve.runze.SY03B-T06", &cmd)
        .unwrap();
    assert_eq!(bytes, b"/1A3000R\r\n");

    // Decode a response
    let props = adapter
        .decode_response("syringe_pump_with_valve.runze.SY03B-T06", b"`3000\n")
        .unwrap();
    assert_eq!(props["status"], "Idle");
    assert_eq!(props["position"], 12.5);

    // T08 should also work
    let matched = adapter.match_hardware("syringe_pump_with_valve.runze.SY03B-T08");
    assert!(matched.is_some(), "Should match Runze SY03B-T08");
}
