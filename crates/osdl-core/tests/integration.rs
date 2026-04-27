use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::protocol::DeviceCommand;

#[test]
fn test_load_registry() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
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
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
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
    let adapters: Vec<Box<dyn ProtocolAdapter>> = vec![Box::new(UniLabOsAdapter::new(DriverRegistry::with_builtins()))];
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
        transport_id: "node-01".into(),
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

    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
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

// === Laiyu XYZ Pipette Station ===

#[test]
fn test_laiyu_xyz_registry_load() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // All 3 axes + pipette should be loaded
    for dt in [
        "stepper_motor.laiyu_xyz.X",
        "stepper_motor.laiyu_xyz.Y",
        "stepper_motor.laiyu_xyz.Z",
        "pipette.sopa.YYQ",
    ] {
        let m = adapter.match_hardware(dt);
        assert!(m.is_some(), "Should match {}", dt);
    }

    // X axis should have the right actions
    let m = adapter.match_hardware("stepper_motor.laiyu_xyz.X").unwrap();
    let names: Vec<&str> = m.actions.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"get_status"));
    assert!(names.contains(&"move_to_position"));
    assert!(names.contains(&"start_motion"));
    assert!(names.contains(&"enable"));
    assert!(names.contains(&"home"));
}

#[test]
fn test_laiyu_xyz_encode_via_adapter() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // X axis (slave 1): get_status → Modbus read registers
    let cmd = DeviceCommand {
        command_id: "t1".into(),
        device_id: "xyz-x".into(),
        action: "get_status".into(),
        params: serde_json::json!({}),
    };
    let bytes = adapter
        .encode_command("stepper_motor.laiyu_xyz.X", &cmd)
        .unwrap();
    assert_eq!(bytes[0], 1); // slave 1
    assert_eq!(bytes[1], 0x03); // read registers
    assert_eq!(bytes.len(), 8); // Modbus frame

    // Y axis (slave 2): same command, different slave
    let bytes = adapter
        .encode_command("stepper_motor.laiyu_xyz.Y", &cmd)
        .unwrap();
    assert_eq!(bytes[0], 2); // slave 2

    // Z axis (slave 3)
    let bytes = adapter
        .encode_command("stepper_motor.laiyu_xyz.Z", &cmd)
        .unwrap();
    assert_eq!(bytes[0], 3); // slave 3
}

#[test]
fn test_laiyu_xyz_decode_via_adapter() {
    use osdl_core::driver::util::modbus_rtu;

    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // Simulate X axis status response: standby, pos=2048, speed=0, emergency=0, current=50
    let mut frame = vec![
        0x01, 0x03, 12,
        0x00, 0x00, // status = standby
        0x00, 0x00, // pos_high
        0x08, 0x00, // pos_low = 2048
        0x00, 0x00, // speed
        0x00, 0x00, // emergency
        0x00, 0x32, // current = 50
    ];
    let crc = modbus_rtu::crc16(&frame);
    frame.extend_from_slice(&crc);

    let props = adapter
        .decode_response("stepper_motor.laiyu_xyz.X", &frame)
        .unwrap();
    assert_eq!(props["axis"], "X");
    assert_eq!(props["status"], "standby");
    assert_eq!(props["position_steps"], 2048);

    // Same frame should NOT decode for Y axis (slave 2, but frame has slave 1)
    let props = adapter.decode_response("stepper_motor.laiyu_xyz.Y", &frame);
    assert!(props.is_none(), "Slave 1 frame should not decode for Y (slave 2)");
}

#[test]
fn test_sopa_pipette_via_adapter() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // Aspirate 200 uL
    let cmd = DeviceCommand {
        command_id: "t2".into(),
        device_id: "pip".into(),
        action: "aspirate".into(),
        params: serde_json::json!({"volume": 200.0}),
    };
    let bytes = adapter
        .encode_command("pipette.sopa.YYQ", &cmd)
        .unwrap();
    assert!(bytes.starts_with(b"/4P200E"));

    // Query tip
    let cmd = DeviceCommand {
        command_id: "t3".into(),
        device_id: "pip".into(),
        action: "query_tip".into(),
        params: serde_json::json!({}),
    };
    let bytes = adapter
        .encode_command("pipette.sopa.YYQ", &cmd)
        .unwrap();
    assert!(bytes.starts_with(b"/4Q28E"));
}

// === ChinWe Separator Station ===

#[test]
fn test_chinwe_registry_load() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // All 6 devices should be loaded
    for dt in [
        "syringe_pump.chinwe.pump1",
        "syringe_pump.chinwe.pump2",
        "syringe_pump.chinwe.pump3",
        "stepper_motor.chinwe.emm4",
        "stepper_motor.chinwe.emm5",
        "sensor.chinwe.xkc",
    ] {
        let m = adapter.match_hardware(dt);
        assert!(m.is_some(), "Should match {}", dt);
    }
}

#[test]
fn test_chinwe_pump_line_ending() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // ChinWe pump uses \r (not \r\n)
    let cmd = DeviceCommand {
        command_id: "t4".into(),
        device_id: "p1".into(),
        action: "initialize".into(),
        params: serde_json::json!({}),
    };
    let bytes = adapter
        .encode_command("syringe_pump.chinwe.pump1", &cmd)
        .unwrap();
    assert_eq!(bytes, b"/1ZR\r");

    // Pump 2 has address "2"
    let bytes = adapter
        .encode_command("syringe_pump.chinwe.pump2", &cmd)
        .unwrap();
    assert_eq!(bytes, b"/2ZR\r");

    // Pump 3 has address "3"
    let bytes = adapter
        .encode_command("syringe_pump.chinwe.pump3", &cmd)
        .unwrap();
    assert_eq!(bytes, b"/3ZR\r");
}

#[test]
fn test_chinwe_emm_encode_via_adapter() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // Motor 4: enable
    let cmd = DeviceCommand {
        command_id: "t5".into(),
        device_id: "m4".into(),
        action: "enable".into(),
        params: serde_json::json!({"enable": true}),
    };
    let bytes = adapter
        .encode_command("stepper_motor.chinwe.emm4", &cmd)
        .unwrap();
    assert_eq!(bytes, vec![4, 0xF3, 0xAB, 1, 0, 0x6B]);

    // Motor 5: enable → device_id should be 5
    let bytes = adapter
        .encode_command("stepper_motor.chinwe.emm5", &cmd)
        .unwrap();
    assert_eq!(bytes[0], 5);

    // Motor 4: stop
    let cmd = DeviceCommand {
        command_id: "t6".into(),
        device_id: "m4".into(),
        action: "stop".into(),
        params: serde_json::json!({}),
    };
    let bytes = adapter
        .encode_command("stepper_motor.chinwe.emm4", &cmd)
        .unwrap();
    assert_eq!(bytes, vec![4, 0xFE, 0x98, 0, 0x6B]);
}

#[test]
fn test_chinwe_emm_decode_routing() {
    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // Position response from device 4
    let response = vec![4, 0x32, 0, 0x00, 0x00, 0x03, 0xE8, 0x6B];

    // Should decode for motor 4
    let props = adapter
        .decode_response("stepper_motor.chinwe.emm4", &response)
        .unwrap();
    assert_eq!(props["position"], 1000);

    // Should NOT decode for motor 5 (wrong device_id)
    let props = adapter.decode_response("stepper_motor.chinwe.emm5", &response);
    assert!(props.is_none());
}

#[test]
fn test_chinwe_xkc_via_adapter() {
    use osdl_core::driver::util::modbus_rtu;

    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // Encode read_level → Modbus read registers from slave 6
    let cmd = DeviceCommand {
        command_id: "t7".into(),
        device_id: "xkc".into(),
        action: "read_level".into(),
        params: serde_json::json!({}),
    };
    let bytes = adapter
        .encode_command("sensor.chinwe.xkc", &cmd)
        .unwrap();
    assert_eq!(bytes[0], 6); // slave 6
    assert_eq!(bytes[1], 0x03); // read registers

    // Decode response: RSSI=500 (above threshold 300 → level=true)
    let mut frame = vec![0x06, 0x03, 0x04, 0x00, 0x00, 0x01, 0xF4];
    let crc = modbus_rtu::crc16(&frame);
    frame.extend_from_slice(&crc);

    let props = adapter
        .decode_response("sensor.chinwe.xkc", &frame)
        .unwrap();
    assert_eq!(props["rssi"], 500);
    assert_eq!(props["level"], true);
}

#[test]
fn test_shared_bus_decode_isolation() {
    use osdl_core::driver::util::modbus_rtu;

    let mut adapter = UniLabOsAdapter::new(DriverRegistry::with_builtins());
    adapter.load_registry("../../registry/unilabos").unwrap();

    // A Modbus response from slave 1 (Laiyu X axis)
    let mut frame = vec![
        0x01, 0x03, 12,
        0x00, 0x00, 0x00, 0x00, 0x08, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x32,
    ];
    let crc = modbus_rtu::crc16(&frame);
    frame.extend_from_slice(&crc);

    // Should decode for X (slave 1) but not Y (slave 2), Z (slave 3), or XKC (slave 6)
    assert!(adapter.decode_response("stepper_motor.laiyu_xyz.X", &frame).is_some());
    assert!(adapter.decode_response("stepper_motor.laiyu_xyz.Y", &frame).is_none());
    assert!(adapter.decode_response("stepper_motor.laiyu_xyz.Z", &frame).is_none());
    assert!(adapter.decode_response("sensor.chinwe.xkc", &frame).is_none());

    // An Emm frame for device 4 should not decode as Modbus
    let emm_frame = vec![4, 0x32, 0, 0x00, 0x00, 0x03, 0xE8, 0x6B];
    assert!(adapter.decode_response("stepper_motor.laiyu_xyz.X", &emm_frame).is_none());
    assert!(adapter.decode_response("stepper_motor.chinwe.emm4", &emm_frame).is_some());
    assert!(adapter.decode_response("stepper_motor.chinwe.emm5", &emm_frame).is_none());
}
