use crate::adapter::PlatformAdapter;
use crate::config::OsdlConfig;
use crate::event::OsdlEvent;
use crate::mqtt::MqttBridge;
use crate::protocol::*;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum OsdlStatus {
    Disconnected,
    Connecting,
    Connected { broker: String, device_count: usize },
    Error { message: String },
}

pub struct OsdlEngine {
    config: OsdlConfig,
    adapters: Vec<Box<dyn PlatformAdapter>>,
    devices: Arc<RwLock<HashMap<String, Device>>>,

    event_tx: mpsc::UnboundedSender<OsdlEvent>,
    event_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<OsdlEvent>>>>,

    status_tx: watch::Sender<OsdlStatus>,
    status_rx: watch::Receiver<OsdlStatus>,
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,

    mqtt: Option<MqttBridge>,
}

impl OsdlEngine {
    pub fn new(config: OsdlConfig, adapters: Vec<Box<dyn PlatformAdapter>>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = watch::channel(OsdlStatus::Disconnected);
        let (stop_tx, stop_rx) = watch::channel(false);

        Self {
            config,
            adapters,
            devices: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
            status_tx,
            status_rx,
            stop_tx,
            stop_rx,
            mqtt: None,
        }
    }

    /// Take the event receiver. The host calls this once to forward events.
    pub fn take_event_rx(
        &self,
    ) -> Arc<Mutex<Option<mpsc::UnboundedReceiver<OsdlEvent>>>> {
        self.event_rx.clone()
    }

    pub fn status_rx(&self) -> watch::Receiver<OsdlStatus> {
        self.status_rx.clone()
    }

    pub fn stop_handle(&self) -> watch::Sender<bool> {
        self.stop_tx.clone()
    }

    /// Main loop: connect to MQTT, start adapters, process messages.
    pub async fn run(&mut self) {
        let _ = self.status_tx.send(OsdlStatus::Connecting);

        let bridge = MqttBridge::new(&self.config.mqtt);
        let client = bridge.client.clone();
        self.mqtt = Some(bridge);

        // Load registries and start adapters
        for (i, adapter_cfg) in self.config.adapters.iter().enumerate() {
            if let Some(ref path) = adapter_cfg.registry_path {
                if let Some(adapter) = self.adapters.get_mut(i) {
                    if let Err(e) = adapter.load_registry(path) {
                        log::error!("Failed to load registry for {}: {}", adapter.platform(), e);
                    }
                }
            }
            if let Some(adapter) = self.adapters.get(i) {
                if let Err(e) = adapter.start(&client).await {
                    log::error!("Failed to start adapter {}: {}", adapter.platform(), e);
                }
            }
        }

        // Populate initial device list from adapters
        {
            let mut devices = self.devices.write().await;
            for adapter in &self.adapters {
                for dev in adapter.devices() {
                    devices.insert(dev.id.clone(), dev);
                }
            }
        }

        let device_count = self.devices.read().await.len();
        let broker = format!("{}:{}", self.config.mqtt.host, self.config.mqtt.port);
        let _ = self.status_tx.send(OsdlStatus::Connected {
            broker: broker.clone(),
            device_count,
        });
        log::info!("OSDL connected to {} with {} devices", broker, device_count);

        // Event loop: process MQTT messages
        let mqtt = self.mqtt.as_mut().unwrap();
        let mut stop_rx = self.stop_rx.clone();

        loop {
            tokio::select! {
                event = mqtt.eventloop.poll() => {
                    match event {
                        Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish))) => {
                            for adapter in &self.adapters {
                                if let Some(osdl_event) = adapter.parse_message(&publish.topic, &publish.payload) {
                                    // Update local device cache
                                    if let OsdlEvent::DeviceStatus(ref status) = osdl_event {
                                        let mut devices = self.devices.write().await;
                                        if let Some(dev) = devices.get_mut(&status.device_id) {
                                            dev.properties = status.properties.clone();
                                        }
                                    }
                                    let _ = self.event_tx.send(osdl_event);
                                    break;
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::error!("MQTT error: {}", e);
                            let _ = self.status_tx.send(OsdlStatus::Error { message: e.to_string() });
                            break;
                        }
                    }
                }
                _ = stop_rx.changed() => {
                    log::info!("OSDL engine stopping");
                    break;
                }
            }
        }

        // Cleanup
        for adapter in &self.adapters {
            adapter.stop().await;
        }
        let _ = self.status_tx.send(OsdlStatus::Disconnected);
    }

    // === Request-response API (called by host) ===

    pub async fn list_devices(&self) -> Vec<Device> {
        self.devices.read().await.values().cloned().collect()
    }

    pub async fn get_device(&self, device_id: &str) -> Option<Device> {
        self.devices.read().await.get(device_id).cloned()
    }

    pub async fn send_command(&self, cmd: DeviceCommand) -> Result<CommandResult, String> {
        let device = self.devices.read().await.get(&cmd.device_id).cloned();
        let device = device.ok_or_else(|| format!("unknown device: {}", cmd.device_id))?;

        let mqtt = self
            .mqtt
            .as_ref()
            .ok_or("MQTT not connected")?;

        for adapter in &self.adapters {
            if adapter.platform() == device.adapter {
                return adapter.dispatch_command(&mqtt.client, &cmd).await;
            }
        }

        Err(format!("no adapter for platform: {}", device.adapter))
    }

    pub fn status(&self) -> OsdlStatus {
        self.status_rx.borrow().clone()
    }
}
