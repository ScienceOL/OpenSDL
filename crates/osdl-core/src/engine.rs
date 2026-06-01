use crate::adapter::ProtocolAdapter;
use crate::config::OsdlConfig;
use crate::event::OsdlEvent;
use crate::media::mediamtx::MediamtxProcess;
use crate::mqtt::MqttBridge;
use crate::protocol::*;
use crate::store::EventStore;
#[cfg(feature = "espnow")]
use crate::transport::espnow_dongle::{
    transport_id_for as espnow_transport_id, EspNowNodeTransport, EspNowDongleClient, Mac,
    RegEvent,
};
use crate::transport::mqtt_serial::MqttSerialTransport;
use crate::transport::{Transport, TransportRx};

use rumqttc::AsyncClient;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, watch, RwLock};

/// Buffer size for the broadcast events channel. Slow subscribers that fall
/// further behind than this start receiving `RecvError::Lagged(n)` so they
/// can resync rather than wedging the engine.
const EVENT_BROADCAST_CAP: usize = 256;

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

/// Cheap-to-clone, `Send + Sync` view of a running engine.
///
/// `EngineHandle` carries the shared state and channels needed to *interact*
/// with the engine — list devices, send commands, subscribe to events,
/// request shutdown — but **not** the per-loop receivers (`run` consumes
/// those once). The gRPC server, the CLI, the orchestrator, and tests all
/// hold their own clones of this handle. The engine task holds one too and
/// operates on the same shared state.
#[derive(Clone)]
pub struct EngineHandle {
    config: Arc<OsdlConfig>,
    adapters: Arc<Vec<Box<dyn ProtocolAdapter>>>,
    store: Option<Arc<EventStore>>,

    /// Connected nodes (node_id → Node). Specific to MQTT serial transport.
    nodes: Arc<RwLock<HashMap<String, Node>>>,
    /// All devices regardless of transport type (device_id → Device).
    devices: Arc<RwLock<HashMap<String, Device>>>,
    /// Active transports (transport_id → Transport).
    transports: Arc<RwLock<HashMap<String, Arc<dyn Transport>>>>,

    /// Sender for transports to push received bytes back to the engine.
    transport_rx_tx: mpsc::UnboundedSender<TransportRx>,
    /// External command injection — drop a `DeviceCommand` in here and the
    /// engine's main loop dispatches it via `send_command`.
    cmd_inject_tx: mpsc::UnboundedSender<DeviceCommand>,

    /// Multi-consumer event channel. Each subscriber gets its own
    /// `broadcast::Receiver`. Subscribers that fall behind get
    /// `RecvError::Lagged(n)`.
    events_tx: broadcast::Sender<OsdlEvent>,

    status_tx: watch::Sender<OsdlStatus>,
    status_rx: watch::Receiver<OsdlStatus>,
    stop_tx: watch::Sender<bool>,
}

impl EngineHandle {
    // === Inspection ===

    pub async fn list_devices(&self) -> Vec<Device> {
        self.devices.read().await.values().cloned().collect()
    }

    pub async fn get_device(&self, device_id: &str) -> Option<Device> {
        self.devices.read().await.get(device_id).cloned()
    }

    pub async fn list_nodes(&self) -> Vec<Node> {
        self.nodes.read().await.values().cloned().collect()
    }

    /// Get a reference to the event store (for querying logs).
    pub fn store(&self) -> Option<&Arc<EventStore>> {
        self.store.as_ref()
    }

    pub fn config(&self) -> &OsdlConfig {
        &self.config
    }

    pub fn status(&self) -> OsdlStatus {
        self.status_rx.borrow().clone()
    }

    pub fn status_rx(&self) -> watch::Receiver<OsdlStatus> {
        self.status_rx.clone()
    }

    /// Subscribe to engine events. Each call returns a fresh receiver — only
    /// events sent *after* this call land in it.
    pub fn subscribe_events(&self) -> broadcast::Receiver<OsdlEvent> {
        self.events_tx.subscribe()
    }

    // === Channels ===

    /// Sender for injecting commands into the engine's main loop.
    pub fn command_sender(&self) -> mpsc::UnboundedSender<DeviceCommand> {
        self.cmd_inject_tx.clone()
    }

    /// Sender for custom transports to push received bytes into the engine.
    pub fn transport_rx_sender(&self) -> mpsc::UnboundedSender<TransportRx> {
        self.transport_rx_tx.clone()
    }

    /// Request the engine's main loop to stop. The loop will drain pending
    /// work, shut down media + ESP-NOW gateways, and return from `run()`.
    pub fn request_stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Stop signaller (kept for callers that want to retain the watch
    /// sender directly; most should prefer `request_stop`).
    pub fn stop_handle(&self) -> watch::Sender<bool> {
        self.stop_tx.clone()
    }

    // === Mutation ===

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
    /// Unlike MQTT node registration which auto-discovers devices, this
    /// method explicitly registers a device with its transport. The device's
    /// `transport_id` must match a previously registered transport.
    pub async fn register_device(&self, device: Device) -> Result<(), String> {
        let transport_id = device.transport_id.clone();
        let device_id = device.id.clone();

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

        let bytes = self
            .adapters
            .iter()
            .find(|a| a.platform() == adapter_name)
            .ok_or_else(|| format!("no adapter for platform: {}", adapter_name))?
            .encode_command(&device_type, &cmd)?;

        if let Some(store) = &self.store {
            store.log_serial(&transport_id, "tx", &bytes);
        }

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

    // === Internal helpers (used by engine loop and `register_device`) ===

    /// Emit an event: log to store + broadcast to subscribers.
    pub(crate) fn emit(&self, event: OsdlEvent) {
        if let Some(store) = &self.store {
            store.log_event(&event);
        }
        // No subscribers is the steady state for a freshly-booted engine —
        // not an error.
        let _ = self.events_tx.send(event);
    }

    /// Update the broadcast status snapshot from current state.
    pub(crate) async fn broadcast_status(&self) {
        let broker = self
            .config
            .mqtt
            .as_ref()
            .map(|c| format!("{}:{}", c.host, c.port))
            .unwrap_or_else(|| "mqtt-disabled".into());
        let node_count = self.nodes.read().await.len();
        let device_count = self.devices.read().await.len();
        let _ = self.status_tx.send(OsdlStatus::Connected {
            broker,
            node_count,
            device_count,
        });
    }
}

pub struct OsdlEngine {
    handle: EngineHandle,

    /// Loop-only receivers — consumed once by `run()`.
    transport_rx_rx: Option<mpsc::UnboundedReceiver<TransportRx>>,
    cmd_inject_rx: Option<mpsc::UnboundedReceiver<DeviceCommand>>,

    /// MQTT client, populated in `run()` when MQTT is enabled. Used by
    /// `handle_node_register` to construct an `MqttSerialTransport`.
    mqtt_client: Option<AsyncClient>,

    /// ESP-NOW dongle boards kept alive for the lifetime of the engine.
    /// Indexed by serial port path; each owns a serial read loop.
    #[cfg(feature = "espnow")]
    espnow_dongles: Arc<RwLock<HashMap<String, Arc<EspNowDongleClient>>>>,
    /// REG events from all dongles, multiplexed into one stream for the
    /// main select! loop to react to.
    #[cfg(feature = "espnow")]
    espnow_reg_tx: mpsc::UnboundedSender<(Arc<EspNowDongleClient>, RegEvent)>,
    #[cfg(feature = "espnow")]
    espnow_reg_rx: Option<mpsc::UnboundedReceiver<(Arc<EspNowDongleClient>, RegEvent)>>,
}

impl OsdlEngine {
    /// Build an engine. Loads adapter registries up-front so the resulting
    /// adapters can be shared (read-only) via `Arc` between the engine loop
    /// and `EngineHandle` clones. Registry load failures are logged but
    /// don't fail construction — that matches the previous in-`run()`
    /// behavior.
    pub fn new(config: OsdlConfig, mut adapters: Vec<Box<dyn ProtocolAdapter>>) -> Self {
        for adapter_cfg in &config.adapters {
            if let Some(ref path) = adapter_cfg.registry_path {
                for adapter in adapters.iter_mut() {
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

        let (events_tx, _) = broadcast::channel(EVENT_BROADCAST_CAP);
        let (status_tx, status_rx) = watch::channel(OsdlStatus::Disconnected);
        let (stop_tx, _) = watch::channel(false);
        let (transport_rx_tx, transport_rx_rx) = mpsc::unbounded_channel();
        let (cmd_inject_tx, cmd_inject_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "espnow")]
        let (espnow_reg_tx, espnow_reg_rx) = mpsc::unbounded_channel();

        let handle = EngineHandle {
            config: Arc::new(config),
            adapters: Arc::new(adapters),
            store: None,
            nodes: Arc::new(RwLock::new(HashMap::new())),
            devices: Arc::new(RwLock::new(HashMap::new())),
            transports: Arc::new(RwLock::new(HashMap::new())),
            transport_rx_tx,
            cmd_inject_tx,
            events_tx,
            status_tx,
            status_rx,
            stop_tx,
        };

        Self {
            handle,
            transport_rx_rx: Some(transport_rx_rx),
            cmd_inject_rx: Some(cmd_inject_rx),
            mqtt_client: None,
            #[cfg(feature = "espnow")]
            espnow_dongles: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "espnow")]
            espnow_reg_tx,
            #[cfg(feature = "espnow")]
            espnow_reg_rx: Some(espnow_reg_rx),
        }
    }

    /// Attach an event store for safety logging. Must be called before `run()`.
    pub fn with_store(mut self, store: EventStore) -> Self {
        self.handle.store = Some(Arc::new(store));
        self
    }

    /// Cheap-to-clone handle exposing inspection / command / event APIs.
    /// Most callers (gRPC server, orchestrator, CLI) talk to the engine
    /// through this handle rather than `&OsdlEngine` directly.
    pub fn handle(&self) -> EngineHandle {
        self.handle.clone()
    }

    // === Convenience pass-throughs (kept for ergonomics; identical to `handle()` calls) ===

    pub fn store(&self) -> Option<&Arc<EventStore>> {
        self.handle.store()
    }

    pub fn command_sender(&self) -> mpsc::UnboundedSender<DeviceCommand> {
        self.handle.command_sender()
    }

    pub fn transport_rx_sender(&self) -> mpsc::UnboundedSender<TransportRx> {
        self.handle.transport_rx_sender()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<OsdlEvent> {
        self.handle.subscribe_events()
    }

    pub fn status_rx(&self) -> watch::Receiver<OsdlStatus> {
        self.handle.status_rx()
    }

    pub fn status(&self) -> OsdlStatus {
        self.handle.status()
    }

    pub fn stop_handle(&self) -> watch::Sender<bool> {
        self.handle.stop_handle()
    }

    pub async fn list_devices(&self) -> Vec<Device> {
        self.handle.list_devices().await
    }

    pub async fn get_device(&self, device_id: &str) -> Option<Device> {
        self.handle.get_device(device_id).await
    }

    pub async fn list_nodes(&self) -> Vec<Node> {
        self.handle.list_nodes().await
    }

    pub async fn register_transport(&self, id: String, transport: Arc<dyn Transport>) {
        self.handle.register_transport(id, transport).await
    }

    pub async fn register_device(&self, device: Device) -> Result<(), String> {
        self.handle.register_device(device).await
    }

    pub async fn send_command(&self, cmd: DeviceCommand) -> Result<CommandResult, String> {
        self.handle.send_command(cmd).await
    }

    /// Main loop: connect to MQTT, subscribe to node topics, process messages.
    pub async fn run(&mut self) {
        let _ = self.handle.status_tx.send(OsdlStatus::Connecting);

        // Connect MQTT if configured. `None` means the MQTT serial bridge is
        // disabled — the engine runs without broker/subscriptions and the
        // MQTT arm of the select! loop stays pending forever.
        let mut eventloop = if let Some(mqtt_cfg) = &self.handle.config.mqtt {
            let bridge = MqttBridge::new(mqtt_cfg);
            let (client, eventloop) = bridge.split();
            self.mqtt_client = Some(client.clone());

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

            let broker = format!("{}:{}", mqtt_cfg.host, mqtt_cfg.port);
            log::info!("OSDL engine connected to MQTT broker {}", broker);
            Some(eventloop)
        } else {
            log::info!("OSDL engine running without MQTT (ESP-NOW / direct only)");
            None
        };

        // Emit initial Connected status (broker description reflects MQTT mode).
        let _ = self.handle.status_tx.send(OsdlStatus::Connected {
            broker: self
                .handle
                .config
                .mqtt
                .as_ref()
                .map(|c| format!("{}:{}", c.host, c.port))
                .unwrap_or_else(|| "mqtt-disabled".into()),
            node_count: 0,
            device_count: 0,
        });

        // Start configured ESP-NOW dongles (USB-CDC). Each dongle owns a
        // serial read loop and emits REG events for registration-driven
        // device discovery.
        #[cfg(feature = "espnow")]
        self.start_espnow_dongles().await;

        // Start the media gateway (mediamtx) when any media sources are
        // configured. Failures here don't abort the engine — devices and
        // commands keep working without streams.
        let mut media_proc = self.start_media_gateway().await;
        // Periodic poll of mediamtx liveness. The interval is cheap (a
        // single try_wait) and 5s catches a crash within a useful window.
        let mut media_health = tokio::time::interval(std::time::Duration::from_secs(5));
        media_health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut stop_rx = self.handle.stop_tx.subscribe();
        // `watch::subscribe()` marks the *current* value as already-seen,
        // so a `request_stop()` issued before we get here would be lost
        // and run() would block on the first `changed()`. Bail early in
        // that case.
        if *stop_rx.borrow() {
            log::info!("OSDL engine stop already requested before run() entered the loop");
            let _ = self.handle.status_tx.send(OsdlStatus::Disconnected);
            return;
        }
        let mut transport_rx = self.transport_rx_rx.take();
        let mut cmd_inject_rx = self.cmd_inject_rx.take();
        #[cfg(feature = "espnow")]
        let mut espnow_reg_rx = self.espnow_reg_rx.take();

        loop {
            // The ESP-NOW REG arm can't be conditionally included inside a
            // single `tokio::select!` (cfg attrs on arms aren't supported), so
            // the loop body is duplicated per feature flag.
            #[cfg(feature = "espnow")]
            {
                tokio::select! {
                    event = async {
                        match eventloop.as_mut() {
                            Some(el) => el.poll().await.map(Some),
                            None => std::future::pending().await,
                        }
                    } => {
                        match event {
                            Ok(Some(rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish)))) => {
                                self.handle_mqtt_message(&publish.topic, &publish.payload).await;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                log::error!("MQTT error: {}", e);
                                let _ = self.handle.status_tx.send(OsdlStatus::Error {
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
                    Some(cmd) = async {
                        match cmd_inject_rx.as_mut() {
                            Some(r) => r.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match self.handle.send_command(cmd).await {
                            Ok(res) => log::info!("injected command dispatched: {:?}", res),
                            Err(e)  => log::warn!("injected command failed: {}", e),
                        }
                    }
                    _ = media_health.tick(), if media_proc.is_some() => {
                        if let Some(p) = media_proc.as_mut() {
                            if let Some(status) = p.try_exited() {
                                let reason = format!(
                                    "mediamtx exited unexpectedly (status={status})"
                                );
                                log::error!("{reason}");
                                self.handle.emit(OsdlEvent::MediaGatewayDown { reason });
                                media_proc = None;
                            }
                        }
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
                    event = async {
                        match eventloop.as_mut() {
                            Some(el) => el.poll().await.map(Some),
                            None => std::future::pending().await,
                        }
                    } => {
                        match event {
                            Ok(Some(rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish)))) => {
                                self.handle_mqtt_message(&publish.topic, &publish.payload).await;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                log::error!("MQTT error: {}", e);
                                let _ = self.handle.status_tx.send(OsdlStatus::Error {
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
                    Some(cmd) = async {
                        match cmd_inject_rx.as_mut() {
                            Some(r) => r.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match self.handle.send_command(cmd).await {
                            Ok(res) => log::info!("injected command dispatched: {:?}", res),
                            Err(e)  => log::warn!("injected command failed: {}", e),
                        }
                    }
                    _ = media_health.tick(), if media_proc.is_some() => {
                        if let Some(p) = media_proc.as_mut() {
                            if let Some(status) = p.try_exited() {
                                let reason = format!(
                                    "mediamtx exited unexpectedly (status={status})"
                                );
                                log::error!("{reason}");
                                self.handle.emit(OsdlEvent::MediaGatewayDown { reason });
                                media_proc = None;
                            }
                        }
                    }
                    _ = stop_rx.changed() => {
                        log::info!("OSDL engine stopping");
                        break;
                    }
                }
            }
        }

        // Explicitly stop every ESP-NOW dongle so the read tasks (which hold
        // `Arc<EspNowDongleClient>`) can drop — otherwise they'd live on
        // until their serial port EOFs.
        #[cfg(feature = "espnow")]
        self.stop_espnow_dongles().await;

        if let Some(p) = media_proc.take() {
            p.shutdown().await;
        }

        let _ = self.handle.status_tx.send(OsdlStatus::Disconnected);
    }

    /// Spawn mediamtx if any media sources are configured. Emits
    /// `MediaSourceOnline` for each source so the host learns the URLs.
    /// Returns the process handle so the caller can shut it down.
    async fn start_media_gateway(&self) -> Option<MediamtxProcess> {
        let cfg = &self.handle.config;
        if cfg.media_sources.is_empty() {
            return None;
        }

        // Validate all sources before spawning. Catches misconfigurations
        // like remote_rtmp without a transcode path that would otherwise
        // silently produce dead playback URLs.
        for src in &cfg.media_sources {
            if let Err(reason) = src.validate() {
                log::error!("Media source rejected: {reason}");
                self.handle.emit(OsdlEvent::MediaGatewayDown { reason });
                return None;
            }
        }

        let gateway = &cfg.media_gateway;
        let mut all_paths = Vec::new();
        for src in &cfg.media_sources {
            all_paths.extend(src.paths());
        }

        let proc = match MediamtxProcess::spawn(gateway, &all_paths).await {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to start media gateway: {}", e);
                self.handle.emit(OsdlEvent::MediaGatewayDown {
                    reason: e.to_string(),
                });
                return None;
            }
        };

        for src in &cfg.media_sources {
            let endpoints = src.endpoints(&gateway.advertise_host, &gateway.ports);
            log::info!(
                "Media source online: {} ({} endpoints)",
                src.id(),
                endpoints.len(),
            );
            self.handle.emit(OsdlEvent::MediaSourceOnline {
                id: src.id().to_string(),
                description: src.description().to_string(),
                endpoints,
            });
        }

        Some(proc)
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
            self.handle
                .transports
                .write()
                .await
                .insert(node_id.to_string(), transport);
        }

        // Try to match hardware_id across all adapters
        let mut matched = None;
        for adapter in self.handle.adapters.iter() {
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

        self.handle
            .nodes
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
                role: None,
            };

            log::info!(
                "Device matched: {} -> {} ({})",
                node_id,
                device.id,
                device.device_type
            );

            self.handle
                .devices
                .write()
                .await
                .insert(device_id.clone(), device.clone());
            self.handle.emit(OsdlEvent::DeviceOnline(device));
            self.handle.broadcast_status().await;
        } else {
            log::warn!(
                "No driver found for hardware_id: {} (node: {})",
                reg.hardware_id,
                node_id
            );
            self.handle.emit(OsdlEvent::UnknownNode {
                node_id: node_id.to_string(),
                hardware_id: reg.hardware_id,
            });
        }
    }

    /// Handle heartbeat — mark node as online, reset timeout.
    async fn handle_heartbeat(&self, node_id: &str) {
        let mut nodes = self.handle.nodes.write().await;
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
        if let Some(store) = &self.handle.store {
            store.log_serial(transport_id, "rx", bytes);
        }

        let devices = self.handle.devices.read().await;

        // Collect all devices sharing this transport
        let matching: Vec<_> = devices
            .values()
            .filter(|d| d.transport_id == transport_id)
            .map(|d| (d.id.clone(), d.device_type.clone(), d.adapter.clone()))
            .collect();

        if matching.is_empty() {
            // Fallback: MQTT serial node → device_id mapping (one node = one device)
            let nodes = self.handle.nodes.read().await;
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
        for adapter in self.handle.adapters.iter() {
            if adapter.platform() == adapter_name {
                if let Some(properties) = adapter.decode_response(device_type, bytes) {
                    let status = DeviceStatus {
                        device_id: device_id.to_string(),
                        timestamp: now_millis(),
                        properties,
                    };
                    let mut devices_w = self.handle.devices.write().await;
                    if let Some(dev) = devices_w.get_mut(device_id) {
                        for (k, v) in &status.properties {
                            dev.properties.insert(k.clone(), v.clone());
                        }
                    }
                    self.handle.emit(OsdlEvent::DeviceStatus(status));
                }
                break;
            }
        }
    }

    /// Spin up every dongle in `config.espnow_dongles`, subscribe to their
    /// REG streams, and forward events into the engine's main loop. Replays
    /// any registrations that already arrived before the listener attached.
    #[cfg(feature = "espnow")]
    async fn start_espnow_dongles(&self) {
        for cfg in &self.handle.config.espnow_dongles {
            let client = Arc::new(EspNowDongleClient::new(
                cfg.port.clone(),
                cfg.baud_rate,
                self.handle.transport_rx_tx.clone(),
            ));
            if let Err(e) = client.start().await {
                log::error!("Failed to start ESP-NOW dongle on {}: {}", cfg.port, e);
                continue;
            }
            self.espnow_dongles
                .write()
                .await
                .insert(cfg.port.clone(), client.clone());

            // Pump REG events from this dongle into the engine's unified
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
            // subscribe_reg() — otherwise a fast-booting node could register
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
                "ESP-NOW dongle listening on {} @ {} baud",
                cfg.port,
                cfg.baud_rate
            );
        }
    }

    /// Stop every ESP-NOW dongle this engine started, so their read tasks
    /// can drop and the engine shuts down cleanly. Called automatically at
    /// the end of `run()`.
    #[cfg(feature = "espnow")]
    async fn stop_espnow_dongles(&self) {
        let dongles: Vec<_> = {
            let mut guard = self.espnow_dongles.write().await;
            guard.drain().collect()
        };
        for (port, client) in dongles {
            if let Err(e) = client.stop().await {
                log::warn!("ESP-NOW dongle {} stop failed: {}", port, e);
            }
        }
    }

    /// Handle a REG announcement from an ESP-NOW node.
    ///
    /// Two registration paths:
    ///
    /// 1. **Bus manifest** (`config.buses`): the node's `hardware_id`
    ///    matches a `BusConfig.match_hardware_id`. We register one
    ///    `Device` per entry in `devices`, all sharing the node's
    ///    transport — this is how one ESP-NOW node bridging a shared
    ///    RS-485 bus exposes multiple addressable devices to the engine.
    ///
    /// 2. **Legacy 1:1**: no bus manifest matches. Fall back to the old
    ///    behavior — look up the hardware_id directly in the adapter
    ///    registry and create a single Device keyed on the MAC.
    ///
    /// Idempotent: re-REG (node reboot) is a no-op once transport + any
    /// devices exist.
    #[cfg(feature = "espnow")]
    async fn handle_espnow_reg(&self, client: Arc<EspNowDongleClient>, reg: RegEvent) {
        let RegEvent {
            hardware_id,
            mac,
            is_new,
        } = reg;
        let transport_id = espnow_transport_id(&mac);

        let node: Arc<dyn Transport> = Arc::new(EspNowNodeTransport::new(mac, client.clone()));
        {
            let mut transports = self.handle.transports.write().await;
            transports.entry(transport_id.clone()).or_insert(node);
        }

        if !is_new {
            return;
        }

        // Path 1: bus manifest.
        let bus = self
            .handle
            .config
            .buses
            .iter()
            .find(|b| b.match_hardware_id == hardware_id)
            .cloned();
        if let Some(bus) = bus {
            self.register_bus_devices(&bus, &transport_id, &mac, &hardware_id)
                .await;
            return;
        }

        // Path 2: legacy 1:1 registration.
        let device_id = transport_id.clone();
        if self.handle.devices.read().await.contains_key(&device_id) {
            return;
        }

        let mut matched = None;
        for adapter in self.handle.adapters.iter() {
            if let Some(m) = adapter.match_hardware(&hardware_id) {
                matched = Some((adapter.platform().to_string(), m));
                break;
            }
        }

        if let Some((platform, device_match)) = matched {
            let device = Device {
                id: device_id.clone(),
                transport_id: transport_id.clone(),
                device_type: device_match.device_type,
                adapter: platform,
                description: device_match.description,
                online: true,
                properties: HashMap::new(),
                actions: device_match.actions,
                role: None,
            };
            log::info!(
                "ESP-NOW device matched: hardware_id={} -> {} ({}) MAC {}",
                hardware_id,
                device.id,
                device.device_type,
                mac_hex_flat(&mac),
            );
            self.handle
                .devices
                .write()
                .await
                .insert(device_id.clone(), device.clone());
            self.handle.emit(OsdlEvent::DeviceOnline(device));
            self.handle.broadcast_status().await;
        } else {
            log::warn!(
                "No driver found for ESP-NOW hardware_id: {} (MAC {})",
                hardware_id,
                mac_hex_flat(&mac),
            );
            self.handle.emit(OsdlEvent::UnknownNode {
                node_id: transport_id,
                hardware_id,
            });
        }
    }

    /// Create one Device per entry in a `BusConfig`, all sharing the
    /// node's transport. Skips entries whose `device_type` isn't found in
    /// any loaded adapter (logged as warn — typically a YAML/config typo).
    ///
    /// Device id format: `{transport_id}:{local_id}` (e.g.
    /// `espnow:30EDA0B65B38:pump-1`), so the Agent can address each device
    /// on a shared bus independently.
    #[cfg(feature = "espnow")]
    async fn register_bus_devices(
        &self,
        bus: &crate::config::BusConfig,
        transport_id: &str,
        mac: &Mac,
        hardware_id: &str,
    ) {
        log::info!(
            "ESP-NOW bus matched: hardware_id={} → {} devices on MAC {}",
            hardware_id,
            bus.devices.len(),
            mac_hex_flat(mac),
        );

        for entry in &bus.devices {
            let device_id = format!("{}:{}", transport_id, entry.local_id);
            if self.handle.devices.read().await.contains_key(&device_id) {
                continue;
            }

            let mut matched = None;
            for adapter in self.handle.adapters.iter() {
                if let Some(m) = adapter.match_hardware(&entry.device_type) {
                    matched = Some((adapter.platform().to_string(), m));
                    break;
                }
            }

            let Some((platform, device_match)) = matched else {
                log::warn!(
                    "bus device {} references unknown device_type '{}' — skipping",
                    device_id,
                    entry.device_type,
                );
                continue;
            };

            let description = entry
                .description
                .clone()
                .unwrap_or(device_match.description);

            let device = Device {
                id: device_id.clone(),
                transport_id: transport_id.to_string(),
                device_type: device_match.device_type,
                adapter: platform,
                description,
                online: true,
                properties: HashMap::new(),
                actions: device_match.actions,
                role: entry.role.clone(),
            };
            log::info!(
                "  bus device registered: {} ({}){}",
                device.id,
                device.device_type,
                device
                    .role
                    .as_deref()
                    .map(|r| format!(" role={}", r))
                    .unwrap_or_default(),
            );
            self.handle
                .devices
                .write()
                .await
                .insert(device_id, device.clone());
            self.handle.emit(OsdlEvent::DeviceOnline(device));
        }
        self.handle.broadcast_status().await;
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
