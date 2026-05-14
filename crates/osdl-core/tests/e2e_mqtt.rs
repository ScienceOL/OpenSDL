//! End-to-end integration tests with a real embedded MQTT broker.
//!
//! These tests start an EmbeddedBroker + OsdlEngine, then simulate a child
//! node (ESP32) by publishing registration and serial RX messages via a
//! second MQTT client. This exercises the full flow:
//!   broker ↔ engine ↔ adapter ↔ runze codec ↔ store

use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::adapter::ProtocolAdapter;
use osdl_core::config::{AdapterConfig, MqttConfig, OsdlConfig};
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::event::OsdlEvent;
use osdl_core::{EmbeddedBroker, EventStore, OsdlEngine, OsdlStatus};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::time::Duration;
use tokio::sync::mpsc;

// ── Test Harness ──────────────────────────────────────────────────────

/// Find a free TCP port to avoid conflicts between parallel tests.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Shared test infrastructure: broker + engine + event channel.
struct TestHarness {
    port: u16,
    _broker: EmbeddedBroker,
    event_rx: Option<mpsc::UnboundedReceiver<OsdlEvent>>,
    status_rx: tokio::sync::watch::Receiver<OsdlStatus>,
    stop: tokio::sync::watch::Sender<bool>,
    engine_handle: tokio::task::JoinHandle<()>,
}

impl TestHarness {
    /// Start a broker + engine with the UniLabOS adapter and optional event store.
    async fn start(client_id: &str, with_store: bool) -> Self {
        let port = free_port();
        let broker = EmbeddedBroker::start(port).expect("broker should start");
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = OsdlConfig {
            mqtt: Some(MqttConfig {
                host: "localhost".into(),
                port,
                client_id: client_id.into(),
                keepalive_secs: 5,
            }),
            adapters: vec![AdapterConfig {
                adapter_type: "unilabos".into(),
                registry_path: Some("../../registry/unilabos".into()),
            }],
            espnow_gateways: vec![],
        };

        let adapters: Vec<Box<dyn ProtocolAdapter>> = vec![Box::new(UniLabOsAdapter::new(DriverRegistry::with_builtins()))];
        let mut engine = if with_store {
            OsdlEngine::new(config, adapters).with_store(EventStore::in_memory().unwrap())
        } else {
            OsdlEngine::new(config, adapters)
        };

        let event_rx_arc = engine.take_event_rx();
        let mut status_rx = engine.status_rx();
        let stop = engine.stop_handle();

        let engine_handle = tokio::spawn(async move {
            engine.run().await;
        });

        // Wait for Connected status
        wait_for_status(
            &mut status_rx,
            |s| matches!(s, OsdlStatus::Connected { .. }),
            3000,
        )
        .await;

        let event_rx = event_rx_arc.lock().await.take();

        Self {
            port,
            _broker: broker,
            event_rx,
            status_rx,
            stop,
            engine_handle,
        }
    }

    /// Start a bare broker + engine with no adapters (for simple connect test).
    async fn start_bare(client_id: &str) -> Self {
        let port = free_port();
        let broker = EmbeddedBroker::start(port).expect("broker should start");
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = OsdlConfig {
            mqtt: Some(MqttConfig {
                host: "localhost".into(),
                port,
                client_id: client_id.into(),
                keepalive_secs: 5,
            }),
            adapters: vec![],
            espnow_gateways: vec![],
        };

        let adapters: Vec<Box<dyn ProtocolAdapter>> = vec![];
        let mut engine = OsdlEngine::new(config, adapters);

        let event_rx_arc = engine.take_event_rx();
        let mut status_rx = engine.status_rx();
        let stop = engine.stop_handle();

        let engine_handle = tokio::spawn(async move {
            engine.run().await;
        });

        wait_for_status(
            &mut status_rx,
            |s| matches!(s, OsdlStatus::Connected { .. }),
            3000,
        )
        .await;

        let event_rx = event_rx_arc.lock().await.take();

        Self {
            port,
            _broker: broker,
            event_rx,
            status_rx,
            stop,
            engine_handle,
        }
    }

    /// Create a simulated child node MQTT client connected to this harness.
    async fn child_client(&self, client_id: &str) -> ChildNode {
        let mut opts = MqttOptions::new(client_id, "localhost", self.port);
        opts.set_keep_alive(Duration::from_secs(5));
        let (client, eventloop) = AsyncClient::new(opts, 64);

        let pump = tokio::spawn(async move {
            let mut eventloop = eventloop;
            loop {
                match eventloop.poll().await {
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        ChildNode { client, pump }
    }

    /// Create a child node that captures TX messages published to its serial topic.
    async fn child_client_with_tx_capture(
        &self,
        client_id: &str,
        node_id: &str,
    ) -> (ChildNode, mpsc::UnboundedReceiver<Vec<u8>>) {
        let mut opts = MqttOptions::new(client_id, "localhost", self.port);
        opts.set_keep_alive(Duration::from_secs(5));
        let (client, eventloop) = AsyncClient::new(opts, 64);

        let (tx_msg_tx, tx_msg_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let pump = tokio::spawn(async move {
            let mut eventloop = eventloop;
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if publish.topic.contains("/tx") {
                            let _ = tx_msg_tx.send(publish.payload.to_vec());
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Subscribe to TX topic
        let tx_topic = format!("osdl/serial/{}/tx", node_id);
        client
            .subscribe(tx_topic, QoS::AtLeastOnce)
            .await
            .unwrap();

        (ChildNode { client, pump }, tx_msg_rx)
    }

    /// Take the event receiver (can only be called once).
    fn take_event_rx(&mut self) -> mpsc::UnboundedReceiver<OsdlEvent> {
        self.event_rx.take().expect("event_rx already taken")
    }

    /// Receive next event with timeout.
    async fn recv_event(rx: &mut mpsc::UnboundedReceiver<OsdlEvent>) -> OsdlEvent {
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("should receive event within timeout")
            .expect("channel should not be closed")
    }

    async fn shutdown(self) {
        let _ = self.stop.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.engine_handle).await;
    }
}

/// A simulated child node (ESP32) connected via MQTT.
struct ChildNode {
    client: AsyncClient,
    pump: tokio::task::JoinHandle<()>,
}

impl ChildNode {
    /// Publish a node registration message (like ESP32 does on boot).
    async fn register(&self, node_id: &str, hardware_id: &str, baud_rate: u32) {
        let payload = serde_json::json!({
            "hardware_id": hardware_id,
            "baud_rate": baud_rate,
        });
        self.client
            .publish(
                format!("osdl/nodes/{}/register", node_id),
                QoS::AtLeastOnce,
                true,
                serde_json::to_vec(&payload).unwrap(),
            )
            .await
            .expect("publish registration");
    }

    /// Publish serial RX bytes (simulating device response arriving at ESP32).
    async fn publish_serial_rx(&self, node_id: &str, bytes: &[u8]) {
        self.client
            .publish(
                format!("osdl/serial/{}/rx", node_id),
                QoS::AtLeastOnce,
                false,
                bytes.to_vec(),
            )
            .await
            .unwrap();
    }

    /// Send a heartbeat for a node.
    async fn heartbeat(&self, node_id: &str) {
        self.client
            .publish(
                format!("osdl/nodes/{}/heartbeat", node_id),
                QoS::AtLeastOnce,
                false,
                b"1".to_vec(),
            )
            .await
            .unwrap();
    }

    async fn shutdown(self) {
        self.client.disconnect().await.ok();
        self.pump.abort();
    }
}

/// Wait for a specific engine status with timeout.
async fn wait_for_status(
    rx: &mut tokio::sync::watch::Receiver<OsdlStatus>,
    check: impl Fn(&OsdlStatus) -> bool,
    timeout_ms: u64,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if check(&*rx.borrow()) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::select! {
            _ = rx.changed() => {}
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

const RUNZE_T06: &str = "syringe_pump_with_valve.runze.SY03B-T06";

// ── Tests ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_broker_start_and_engine_connect() {
    let harness = TestHarness::start_bare("osdl-test-connect").await;

    let connected = matches!(&*harness.status_rx.borrow(), OsdlStatus::Connected { .. });
    assert!(connected, "Engine should reach Connected status");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_node_registration_and_device_discovery() {
    let mut harness = TestHarness::start("osdl-test-reg", true).await;
    let mut rx = harness.take_event_rx();

    let child = harness.child_client("pump-01-sim").await;
    child.register("pump-01", RUNZE_T06, 9600).await;

    let event = TestHarness::recv_event(&mut rx).await;

    match &event {
        OsdlEvent::DeviceOnline(device) => {
            assert_eq!(device.transport_id, "pump-01");
            assert_eq!(device.device_type, RUNZE_T06);
            assert_eq!(device.adapter, "unilabos");
            assert!(device.online);
            assert!(!device.actions.is_empty());
            let action_names: Vec<&str> = device.actions.iter().map(|a| a.name.as_str()).collect();
            assert!(action_names.contains(&"initialize"));
            assert!(action_names.contains(&"set_position"));
            assert!(action_names.contains(&"set_valve_position"));
        }
        other => panic!("Expected DeviceOnline, got {:?}", other),
    }

    // Check engine status reflects 1 node + 1 device
    let status = harness.status_rx.borrow().clone();
    match status {
        OsdlStatus::Connected {
            node_count,
            device_count,
            ..
        } => {
            assert_eq!(node_count, 1);
            assert_eq!(device_count, 1);
        }
        other => panic!("Expected Connected, got {:?}", other),
    }

    child.shutdown().await;
    harness.shutdown().await;
}

#[tokio::test]
async fn test_serial_rx_decoding() {
    let mut harness = TestHarness::start("osdl-test-rx", true).await;
    let mut rx = harness.take_event_rx();

    let child = harness.child_client("pump-02-sim").await;

    // Step 1: Register
    child.register("pump-02", RUNZE_T06, 9600).await;
    let event = TestHarness::recv_event(&mut rx).await;
    assert!(matches!(event, OsdlEvent::DeviceOnline(_)));

    // Step 2: Simulate serial response (pump replies with position)
    tokio::time::sleep(Duration::from_millis(100)).await;
    child.publish_serial_rx("pump-02", b"`3000\n").await;

    let event = TestHarness::recv_event(&mut rx).await;
    match event {
        OsdlEvent::DeviceStatus(status) => {
            assert_eq!(status.device_id, "pump-02:syringe_pump_with_valve.runze.SY03B-T06");
            assert_eq!(status.properties["status"], "Idle");
            assert_eq!(status.properties["position"], 12.5);
        }
        other => panic!("Expected DeviceStatus, got {:?}", other),
    }

    child.shutdown().await;
    harness.shutdown().await;
}

#[tokio::test]
async fn test_send_command_publishes_serial_tx() {
    let mut harness = TestHarness::start("osdl-test-cmd", true).await;
    let mut rx = harness.take_event_rx();

    let (child, mut tx_msg_rx) =
        harness.child_client_with_tx_capture("pump-03-sim", "pump-03").await;

    // Register
    child.register("pump-03", RUNZE_T06, 9600).await;
    let _ = TestHarness::recv_event(&mut rx).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Publish TX bytes via a separate client (simulating the engine's TX path)
    let test_pub = harness.child_client("test-publisher").await;
    test_pub
        .client
        .publish(
            "osdl/serial/pump-03/tx",
            QoS::AtLeastOnce,
            false,
            b"/1ZR\r\n".to_vec(),
        )
        .await
        .unwrap();

    // Child should receive the TX bytes
    let received = tokio::time::timeout(Duration::from_secs(3), tx_msg_rx.recv())
        .await
        .expect("should receive TX bytes")
        .expect("channel open");
    assert_eq!(received, b"/1ZR\r\n");

    test_pub.shutdown().await;
    child.shutdown().await;
    harness.shutdown().await;
}

#[tokio::test]
async fn test_unknown_node_event() {
    let mut harness = TestHarness::start("osdl-test-unknown", false).await;
    let mut rx = harness.take_event_rx();

    let child = harness.child_client("mystery-node").await;
    child
        .register("mystery-01", "totally_unknown_device_xyz", 9600)
        .await;

    let event = TestHarness::recv_event(&mut rx).await;
    match event {
        OsdlEvent::UnknownNode {
            node_id,
            hardware_id,
        } => {
            assert_eq!(node_id, "mystery-01");
            assert_eq!(hardware_id, "totally_unknown_device_xyz");
        }
        other => panic!("Expected UnknownNode, got {:?}", other),
    }

    child.shutdown().await;
    harness.shutdown().await;
}

#[tokio::test]
async fn test_heartbeat_keeps_node_online() {
    let mut harness = TestHarness::start("osdl-test-hb", false).await;
    let mut rx = harness.take_event_rx();

    let child = harness.child_client("hb-node").await;
    child.register("hb-pump", RUNZE_T06, 9600).await;

    // Consume DeviceOnline
    let _ = TestHarness::recv_event(&mut rx).await;

    // Send heartbeat
    child.heartbeat("hb-pump").await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let status = harness.status_rx.borrow().clone();
    match status {
        OsdlStatus::Connected { node_count, .. } => {
            assert_eq!(node_count, 1, "Node should still be registered");
        }
        other => panic!("Expected Connected, got {:?}", other),
    }

    child.shutdown().await;
    harness.shutdown().await;
}
