use crate::adapter::ProtocolAdapter;
use crate::config::OsdlConfig;
use crate::event::OsdlEvent;
use crate::mqtt::MqttBridge;
use crate::protocol::*;
use crate::store::EventStore;
#[cfg(feature = "espnow")]
use crate::transport::espnow_gateway::{EspNowChildTransport, EspNowGatewayClient, Mac, RegEvent};
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

    /// ESP-NOW gateway boards kept alive for the lifetime of the engine.
    /// Indexed by serial port path; each owns a serial read loop.
    #[cfg(feature = "espnow")]
    espnow_gateways: Arc<RwLock<HashMap<String, Arc<EspNowGatewayClient>>>>,
    /// REG events from all gateways, multiplexed into one stream for the
    /// main select! loop to react to.
    #[cfg(feature = "espnow")]
    espnow_reg_tx: mpsc::UnboundedSender<(Arc<EspNowGatewayClient>, RegEvent)>,
    #[cfg(feature = "espnow")]
    espnow_reg_rx: Option<mpsc::UnboundedReceiver<(Arc<EspNowGatewayClient>, RegEvent)>>,
}

impl OsdlEngine {
    pub fn new(config: OsdlConfig, adapters: Vec<Box<dyn ProtocolAdapter>>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = watch::channel(OsdlStatus::Disconnected);
        let (stop_tx, stop_rx) = watch::channel(false);
        let (transport_rx_tx, transport_rx_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "espnow")]
        let (espnow_reg_tx, espnow_reg_rx) = mpsc::unbounded_channel();

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
            #[cfg(feature = "espnow")]
            espnow_gateways: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "espnow")]
            espnow_reg_tx,
            #[cfg(feature = "espnow")]
            espnow_reg_rx: Some(espnow_reg_rx),
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
            if let Err(e) = client.subscribe(*topic, rumqttc::QoS::AtLeastOnce).await {
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

        // Start configured ESP-NOW gateways (USB-CDC). Each gateway owns a
        // serial read loop and emits REG events for registration-driven
        // device discovery.
        #[cfg(feature = "espnow")]
        self.start_espnow_gateways().await;

        let mut eventloop = eventloop;
        let mut stop_rx = self.stop_rx.clone();
        let mut transport_rx = self.transport_rx_rx.take();
        #[cfg(feature = "espnow")]
        let mut espnow_reg_rx = self.espnow_reg_rx.take();

        loop {
            // The ESP-NOW REG arm can't be conditionally included inside a
            // single `tokio::select!` (cfg attrs on arms aren't supported), so
            // the loop body is duplicated per feature flag.
            #[cfg(feature = "espnow")]
            {
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
                    Some(rx) = async {
                        match transport_rx.as_mut() {
                            Some(r) => r.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        self.handle_transport_rx(&rx.transport_id, &rx.data).await;
                    }
                    Some((client, reg)) = async {
                        match espnow_reg_rx.as_mut() {
                            Some(r) => r.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        self.handle_espnow_reg(client, reg).await;
                    }
                    _ = stop_rx.changed() => {
                        log::info!("OSDL engine stopping");
                        break;
                    }
                }
            }
            #[cfg(not(feature = "espnow"))]
            {
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

        self.nodes.write().await.insert(node_id.to_string(), node);

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
    ///
    /// For shared buses (multiple devices on one transport), we try decoding
    /// against ALL devices on that transport. Each codec checks the device
    /// address (Modbus slave_id, Emm device_id, etc.) and returns None for
    /// non-matching responses, so only the correct device gets updated.
    async fn handle_transport_rx(&self, transport_id: &str, bytes: &[u8]) {
        // Log raw bytes for forensic replay
        if let Some(store) = &self.store {
            store.log_serial(transport_id, "rx", bytes);
        }

        let devices = self.devices.read().await;

        // Collect all devices sharing this transport
        let matching: Vec<_> = devices
            .values()
            .filter(|d| d.transport_id == transport_id)
            .map(|d| (d.id.clone(), d.device_type.clone(), d.adapter.clone()))
            .collect();

        if matching.is_empty() {
            // Fallback: MQTT serial node → device_id mapping (one node = one device)
            let nodes = self.nodes.read().await;
            if let Some(node) = nodes.get(transport_id) {
                if let Some(ref did) = node.device_id {
                    if let Some(d) = devices.get(did) {
                        let info = (d.id.clone(), d.device_type.clone(), d.adapter.clone());
                        drop(nodes);
                        drop(devices);
                        self.decode_and_update(&info.0, &info.1, &info.2, bytes)
                            .await;
                        return;
                    }
                }
            }
            log::warn!("RX from unknown transport: {}", transport_id);
            return;
        }
        drop(devices);

        // Try decode for each device on this transport
        for (device_id, device_type, adapter_name) in matching {
            self.decode_and_update(&device_id, &device_type, &adapter_name, bytes)
                .await;
        }
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

    /// Spin up every gateway in `config.espnow_gateways`, subscribe to their
    /// REG streams, and forward events into the engine's main loop. Replays
    /// any registrations that already arrived before the listener attached.
    #[cfg(feature = "espnow")]
    async fn start_espnow_gateways(&self) {
        for gw_cfg in &self.config.espnow_gateways {
            let client = Arc::new(EspNowGatewayClient::new(
                gw_cfg.port.clone(),
                gw_cfg.baud_rate,
                self.transport_rx_tx.clone(),
            ));
            if let Err(e) = client.start().await {
                log::error!(
                    "Failed to start ESP-NOW gateway on {}: {}",
                    gw_cfg.port,
                    e
                );
                continue;
            }
            self.espnow_gateways
                .write()
                .await
                .insert(gw_cfg.port.clone(), client.clone());

            // Pump REG events from this gateway into the engine's unified
            // select! loop via the shared espnow_reg channel.
            let mut reg_rx = client.subscribe_reg();
            let forward_tx = self.espnow_reg_tx.clone();
            let forward_client = client.clone();
            tokio::spawn(async move {
                loop {
                    match reg_rx.recv().await {
                        Ok(ev) => {
                            if forward_tx.send((forward_client.clone(), ev)).is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("ESP-NOW REG listener lagged, dropped {} events", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Replay any registrations that arrived between start() and
            // subscribe_reg() — otherwise a fast-booting child could register
            // before we're listening.
            for (hardware_id, mac) in client.known_registrations().await {
                let _ = self.espnow_reg_tx.send((
                    client.clone(),
                    RegEvent {
                        hardware_id,
                        mac,
                        is_new: true,
                    },
                ));
            }

            log::info!(
                "ESP-NOW gateway listening on {} @ {} baud",
                gw_cfg.port,
                gw_cfg.baud_rate
            );
        }
    }

    /// Handle a REG announcement from an ESP-NOW child: match its hardware_id
    /// against known adapters, create a per-child transport keyed on
    /// hardware_id, and register the device so command routing works.
    ///
    /// Idempotent: a re-REG (child rebooting) just refreshes the transport's
    /// route and leaves the existing device record in place.
    #[cfg(feature = "espnow")]
    async fn handle_espnow_reg(&self, client: Arc<EspNowGatewayClient>, reg: RegEvent) {
        let RegEvent { hardware_id, mac, is_new } = reg;

        // Ensure the per-child transport exists. Keyed on hardware_id so the
        // transport id is stable across gateway restarts.
        {
            let mut transports = self.transports.write().await;
            if !transports.contains_key(&hardware_id) {
                let child = Arc::new(EspNowChildTransport::new(
                    hardware_id.clone(),
                    mac,
                    client.clone(),
                ));
                // start() on the child just registers the route inside the
                // gateway; infallible in practice but propagate errors.
                if let Err(e) = child.start().await {
                    log::warn!(
                        "EspNowChildTransport start failed for {}: {}",
                        hardware_id,
                        e
                    );
                }
                transports.insert(hardware_id.clone(), child);
            }
        }

        // Only run adapter match + device creation the first time we see this
        // child. Re-REG shouldn't duplicate the device record.
        if !is_new {
            return;
        }

        let mut matched = None;
        for adapter in &self.adapters {
            if let Some(m) = adapter.match_hardware(&hardware_id) {
                matched = Some((adapter.platform().to_string(), m));
                break;
            }
        }

        // Skip if the device already exists (e.g. re-created after engine
        // restart picked up old state somewhere).
        let device_id = format!("espnow:{}", hardware_id);
        if self.devices.read().await.contains_key(&device_id) {
            let _ = mac; // silence unused in this branch
            return;
        }

        if let Some((platform, device_match)) = matched {
            let device = Device {
                id: device_id.clone(),
                transport_id: hardware_id.clone(),
                device_type: device_match.device_type,
                adapter: platform,
                description: device_match.description,
                online: true,
                properties: HashMap::new(),
                actions: device_match.actions,
            };
            log::info!(
                "ESP-NOW device matched: {} -> {} ({}) MAC {:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                hardware_id,
                device.id,
                device.device_type,
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
            self.devices
                .write()
                .await
                .insert(device_id.clone(), device.clone());
            self.emit(OsdlEvent::DeviceOnline(device));
            self.broadcast_status().await;
        } else {
            log::warn!(
                "No driver found for ESP-NOW hardware_id: {} (MAC {:02X}{:02X}{:02X}{:02X}{:02X}{:02X})",
                hardware_id,
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
            self.emit(OsdlEvent::UnknownNode {
                node_id: format!("espnow:{}", mac_hex_flat(&mac)),
                hardware_id,
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

    /// Register a transport for direct serial / TCP devices.
    ///
    /// Multiple devices can share the same transport (shared bus).
    /// The transport should already be `start()`ed before calling this.
    pub async fn register_transport(&self, id: String, transport: Arc<dyn Transport>) {
        log::info!("Transport registered: {} ({})", id, transport.description());
        self.transports.write().await.insert(id, transport);
    }

    /// Register a device manually (for direct serial / TCP transports).
    ///
    /// Unlike MQTT node registration which auto-discovers devices,
    /// this method explicitly registers a device with its transport.
    /// The device's `transport_id` must match a previously registered transport.
    pub async fn register_device(&self, device: Device) -> Result<(), String> {
        let transport_id = device.transport_id.clone();
        let device_id = device.id.clone();

        // Verify transport exists
        if !self.transports.read().await.contains_key(&transport_id) {
            return Err(format!(
                "transport '{}' not registered — call register_transport() first",
                transport_id
            ));
        }

        self.devices
            .write()
            .await
            .insert(device_id.clone(), device.clone());

        log::info!(
            "Device registered: {} ({}) on transport {}",
            device_id,
            device.device_type,
            transport_id
        );
        self.emit(OsdlEvent::DeviceOnline(device));
        self.broadcast_status().await;
        Ok(())
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

#[cfg(feature = "espnow")]
fn mac_hex_flat(mac: &Mac) -> String {
    format!(
        "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
