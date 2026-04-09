use crate::adapter::ProtocolAdapter;
use crate::config::OsdlConfig;
use crate::event::OsdlEvent;
use crate::mqtt::MqttBridge;
use crate::protocol::*;
use crate::store::EventStore;
use crate::transport::mqtt_serial::MqttSerialTransport;
use crate::transport::{Transport, TransportRx};

use rumqttc::AsyncClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum OsdlStatus {
    Disconnected,
    Connecting,
    Connected {
        broker: String,
        node_count: usize,
        device_count: usize,
    },
    Error {
        message: String,
    },
}

pub struct OsdlEngine {
    config: OsdlConfig,
    adapters: Vec<Box<dyn ProtocolAdapter>>,
    store: Option<Arc<EventStore>>,

    /// Connected child nodes (node_id → Node). Specific to MQTT serial transport.
    nodes: Arc<RwLock<HashMap<String, Node>>>,
    /// All devices regardless of transport type (device_id → Device).
    devices: Arc<RwLock<HashMap<String, Device>>>,
    /// Active transports (transport_id → Transport).
    transports: Arc<RwLock<HashMap<String, Arc<dyn Transport>>>>,

    /// Channel for transports to push received bytes back to the engine.
    transport_rx_tx: mpsc::UnboundedSender<TransportRx>,
    transport_rx_rx: Option<mpsc::UnboundedReceiver<TransportRx>>,

    event_tx: mpsc::UnboundedSender<OsdlEvent>,
    event_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<OsdlEvent>>>>,

    status_tx: watch::Sender<OsdlStatus>,
    status_rx: watch::Receiver<OsdlStatus>,
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,

    mqtt_client: Option<AsyncClient>,
}

impl OsdlEngine {
    pub fn new(config: OsdlConfig, adapters: Vec<Box<dyn ProtocolAdapter>>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = watch::channel(OsdlStatus::Disconnected);
        let (stop_tx, stop_rx) = watch::channel(false);
        let (transport_rx_tx, transport_rx_rx) = mpsc::unbounded_channel();

        Self {
            config,
            adapters,
            store: None,
            nodes: Arc::new(RwLock::new(HashMap::new())),
            devices: Arc::new(RwLock::new(HashMap::new())),
            transports: Arc::new(RwLock::new(HashMap::new())),
            transport_rx_tx,
            transport_rx_rx: Some(transport_rx_rx),
            event_tx,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
            status_tx,
            status_rx,
            stop_tx,
            stop_rx,
            mqtt_client: None,
        }
    }

    /// Attach an event store for safety logging. Must be called before `run()`.
    pub fn with_store(mut self, store: EventStore) -> Self {
        self.store = Some(Arc::new(store));
        self
    }

    /// Get a reference to the event store (for querying logs).
    pub fn store(&self) -> Option<&Arc<EventStore>> {
        self.store.as_ref()
    }

    /// Take the event receiver. The host calls this once to forward events.
    pub fn take_event_rx(&self) -> Arc<Mutex<Option<mpsc::UnboundedReceiver<OsdlEvent>>>> {
        self.event_rx.clone()
    }

    /// Get the transport RX sender (for custom transports to push received bytes).
    pub fn transport_rx_sender(&self) -> mpsc::UnboundedSender<TransportRx> {
        self.transport_rx_tx.clone()
    }

    pub fn status_rx(&self) -> watch::Receiver<OsdlStatus> {
        self.status_rx.clone()
    }

    pub fn stop_handle(&self) -> watch::Sender<bool> {
        self.stop_tx.clone()
    }

    pub fn status(&self) -> OsdlStatus {
        self.status_rx.borrow().clone()
    }

    /// Emit an event: send to channel + log to store.
    fn emit(&self, event: OsdlEvent) {
        if let Some(store) = &self.store {
            store.log_event(&event);
        }
        let _ = self.event_tx.send(event);
    }

    /// Main loop: connect to MQTT, subscribe to node topics, process messages.
    pub async fn run(&mut self) {
        let _ = self.status_tx.send(OsdlStatus::Connecting);

        // Load registries
        for adapter_cfg in &self.config.adapters {
            if let Some(ref path) = adapter_cfg.registry_path {
                for adapter in &mut self.adapters {
                    if adapter.platform() == adapter_cfg.adapter_type {
                        if let Err(e) = adapter.load_registry(path) {
                            log::error!(
                                "Failed to load registry for {}: {}",
                                adapter.platform(),
                                e
                            );
                        }
                    }
                }
            }
        }

        // Connect MQTT — split into client + eventloop for separate ownership
        let bridge = MqttBridge::new(&self.config.mqtt);
        let (client, eventloop) = bridge.split();
        self.mqtt_client = Some(client.clone());

        // Subscribe to node management + serial tunneling topics
        let subs = [
            "osdl/nodes/+/register",
            "osdl/nodes/+/heartbeat",
            "osdl/serial/+/rx",
        ];
        for topic in &subs {
            if let Err(e) = client
                .subscribe(*topic, rumqttc::QoS::AtLeastOnce)
                .await
            {
                log::error!("Failed to subscribe to {}: {}", topic, e);
            }
        }

        let broker = format!("{}:{}", self.config.mqtt.host, self.config.mqtt.port);
        let _ = self.status_tx.send(OsdlStatus::Connected {
            broker: broker.clone(),
            node_count: 0,
            device_count: 0,
        });
        log::info!("OSDL engine connected to {}", broker);

        let mut eventloop = eventloop;
        let mut stop_rx = self.stop_rx.clone();
        let mut transport_rx = self.transport_rx_rx.take();

        loop {
            tokio::select! {
                event = eventloop.poll() => {
                    match event {
                        Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish))) => {
                            self.handle_mqtt_message(&publish.topic, &publish.payload).await;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::error!("MQTT error: {}", e);
                            let _ = self.status_tx.send(OsdlStatus::Error {
                                message: e.to_string(),
                            });
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        }
                    }
                }
                // Receive bytes from non-MQTT transports (direct serial, TCP, etc.)
                Some(rx) = async {
                    match transport_rx.as_mut() {
                        Some(r) => r.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.handle_transport_rx(&rx.transport_id, &rx.data).await;
                }
                _ = stop_rx.changed() => {
                    log::info!("OSDL engine stopping");
                    break;
                }
            }
        }

        let _ = self.status_tx.send(OsdlStatus::Disconnected);
    }

    /// Route incoming MQTT messages to the appropriate handler.
    async fn handle_mqtt_message(&self, topic: &str, payload: &[u8]) {
        if let Some(node_id) = extract_segment(topic, "osdl/nodes/", "/register") {
            self.handle_node_register(&node_id, payload).await;
        } else if let Some(node_id) = extract_segment(topic, "osdl/nodes/", "/heartbeat") {
            self.handle_heartbeat(&node_id).await;
        } else if let Some(node_id) = extract_segment(topic, "osdl/serial/", "/rx") {
            // MQTT serial RX — route through the unified transport handler
            self.handle_transport_rx(&node_id, payload).await;
        }
    }

    /// Child node registered via MQTT: match hardware, create transport + device.
    async fn handle_node_register(&self, node_id: &str, payload: &[u8]) {
        let reg: NodeRegistration = match serde_json::from_slice(payload) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Invalid registration from {}: {}", node_id, e);
                return;
            }
        };

        log::info!(
            "Node registered: {} (hardware={}, baud={})",
            node_id,
            reg.hardware_id,
            reg.baud_rate
        );

        // Create MQTT serial transport for this node
        if let Some(client) = &self.mqtt_client {
            let transport = Arc::new(MqttSerialTransport::new(
                node_id.to_string(),
                client.clone(),
            ));
            self.transports
                .write()
                .await
                .insert(node_id.to_string(), transport);
        }

        // Try to match hardware_id across all adapters
        let mut matched = None;
        for adapter in &self.adapters {
            if let Some(m) = adapter.match_hardware(&reg.hardware_id) {
                matched = Some((adapter.platform().to_string(), m));
                break;
            }
        }

        let device_id = format!("{}:{}", node_id, reg.hardware_id);

        let node = Node {
            node_id: node_id.to_string(),
            hardware_id: reg.hardware_id.clone(),
            baud_rate: reg.baud_rate,
            online: true,
            device_id: matched.as_ref().map(|_| device_id.clone()),
        };

        self.nodes
            .write()
            .await
            .insert(node_id.to_string(), node);

        if let Some((platform, device_match)) = matched {
            let device = Device {
                id: device_id.clone(),
                transport_id: node_id.to_string(),
                device_type: device_match.device_type,
                adapter: platform,
                description: device_match.description,
                online: true,
                properties: HashMap::new(),
                actions: device_match.actions,
            };

            log::info!(
                "Device matched: {} -> {} ({})",
                node_id,
                device.id,
                device.device_type
            );

            self.devices
                .write()
                .await
                .insert(device_id.clone(), device.clone());
            self.emit(OsdlEvent::DeviceOnline(device));
            self.broadcast_status().await;
        } else {
            log::warn!(
                "No driver found for hardware_id: {} (node: {})",
                reg.hardware_id,
                node_id
            );
            self.emit(OsdlEvent::UnknownNode {
                node_id: node_id.to_string(),
                hardware_id: reg.hardware_id,
            });
        }
    }

    /// Handle heartbeat — mark node as online, reset timeout.
    async fn handle_heartbeat(&self, node_id: &str) {
        let mut nodes = self.nodes.write().await;
        if let Some(node) = nodes.get_mut(node_id) {
            node.online = true;
        }
    }

    /// Handle bytes received from any transport (MQTT serial, direct serial, TCP, etc.).
    ///
    /// This is the unified receive path. All transports eventually call this
    /// with their transport_id and the raw bytes from the device.
    async fn handle_transport_rx(&self, transport_id: &str, bytes: &[u8]) {
        // Log raw bytes for forensic replay
        if let Some(store) = &self.store {
            store.log_serial(transport_id, "rx", bytes);
        }

        // Find which device uses this transport
        let devices = self.devices.read().await;
        let (device_id, device_type, adapter_name) = {
            let device = match devices.values().find(|d| d.transport_id == transport_id) {
                Some(d) => d,
                None => {
                    // For MQTT serial, also check via node → device_id mapping
                    let nodes = self.nodes.read().await;
                    if let Some(node) = nodes.get(transport_id) {
                        if let Some(ref did) = node.device_id {
                            if let Some(d) = devices.get(did) {
                                // Found via node mapping
                                let result = (d.id.clone(), d.device_type.clone(), d.adapter.clone());
                                drop(nodes);
                                drop(devices);
                                return self.decode_and_update(&result.0, &result.1, &result.2, bytes).await;
                            }
                        }
                    }
                    log::warn!("RX from unknown transport: {}", transport_id);
                    return;
                }
            };
            (device.id.clone(), device.device_type.clone(), device.adapter.clone())
        };
        drop(devices);

        self.decode_and_update(&device_id, &device_type, &adapter_name, bytes).await;
    }

    /// Decode bytes via adapter and update device state.
    async fn decode_and_update(
        &self,
        device_id: &str,
        device_type: &str,
        adapter_name: &str,
        bytes: &[u8],
    ) {
        for adapter in &self.adapters {
            if adapter.platform() == adapter_name {
                if let Some(properties) = adapter.decode_response(device_type, bytes) {
                    let status = DeviceStatus {
                        device_id: device_id.to_string(),
                        timestamp: now_millis(),
                        properties,
                    };
                    let mut devices_w = self.devices.write().await;
                    if let Some(dev) = devices_w.get_mut(device_id) {
                        for (k, v) in &status.properties {
                            dev.properties.insert(k.clone(), v.clone());
                        }
                    }
                    self.emit(OsdlEvent::DeviceStatus(status));
                }
                break;
            }
        }
    }

    fn broadcast_status(&self) -> impl std::future::Future<Output = ()> + '_ {
        async {
            let broker = format!("{}:{}", self.config.mqtt.host, self.config.mqtt.port);
            let node_count = self.nodes.read().await.len();
            let device_count = self.devices.read().await.len();
            let _ = self.status_tx.send(OsdlStatus::Connected {
                broker,
                node_count,
                device_count,
            });
        }
    }

    // === Request-response API (called by host) ===

    pub async fn list_devices(&self) -> Vec<Device> {
        self.devices.read().await.values().cloned().collect()
    }

    pub async fn get_device(&self, device_id: &str) -> Option<Device> {
        self.devices.read().await.get(device_id).cloned()
    }

    pub async fn list_nodes(&self) -> Vec<Node> {
        self.nodes.read().await.values().cloned().collect()
    }

    /// Send a command to a device via its transport.
    pub async fn send_command(&self, cmd: DeviceCommand) -> Result<CommandResult, String> {
        // Log the command
        if let Some(store) = &self.store {
            store.log_command(&cmd);
        }

        let devices = self.devices.read().await;
        let device = devices
            .get(&cmd.device_id)
            .ok_or_else(|| format!("unknown device: {}", cmd.device_id))?;

        let transport_id = device.transport_id.clone();
        let device_type = device.device_type.clone();
        let adapter_name = device.adapter.clone();
        drop(devices);

        // Find the adapter and encode command to serial bytes
        let bytes = self
            .adapters
            .iter()
            .find(|a| a.platform() == adapter_name)
            .ok_or_else(|| format!("no adapter for platform: {}", adapter_name))?
            .encode_command(&device_type, &cmd)?;

        // Log outgoing bytes
        if let Some(store) = &self.store {
            store.log_serial(&transport_id, "tx", &bytes);
        }

        // Send via the device's transport
        let transports = self.transports.read().await;
        let transport = transports
            .get(&transport_id)
            .ok_or_else(|| format!("no transport for: {}", transport_id))?;

        transport.send(&bytes).await?;

        Ok(CommandResult {
            command_id: cmd.command_id.clone(),
            device_id: cmd.device_id.clone(),
            status: CommandStatus::Pending,
            message: "command sent".into(),
            data: None,
        })
    }
}

/// Extract middle segment from topic: "{prefix}{segment}{suffix}".
fn extract_segment<'a>(topic: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let rest = topic.strip_prefix(prefix)?;
    rest.strip_suffix(suffix)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
