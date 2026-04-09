use crate::adapter::PlatformAdapter;
use crate::event::OsdlEvent;
use crate::protocol::*;
use async_trait::async_trait;
use rumqttc::AsyncClient;
use std::collections::HashMap;

/// Adapter for the UniLabOS device description standard.
///
/// Reads UniLabOS YAML registry files to understand device capabilities,
/// then speaks the UniLabOS MQTT topic/payload convention to communicate
/// with devices directly.
pub struct UniLabOsAdapter {
    gateway_id: String,
    devices: HashMap<String, Device>,
}

impl UniLabOsAdapter {
    pub fn new(gateway_id: &str) -> Self {
        Self {
            gateway_id: gateway_id.to_string(),
            devices: HashMap::new(),
        }
    }

    fn base_topic(&self) -> String {
        format!("unilabos/{}", self.gateway_id)
    }
}

#[async_trait]
impl PlatformAdapter for UniLabOsAdapter {
    fn platform(&self) -> &str {
        "unilabos"
    }

    fn load_registry(&mut self, path: &str) -> Result<(), String> {
        // TODO: walk `path`, parse each .yaml file using UniLabOS registry format,
        // convert to Device structs with ActionSchema derived from action_value_mappings
        // and status_types from the YAML.
        log::info!("Loading UniLabOS registry from: {}", path);
        let _ = path;
        Ok(())
    }

    async fn start(&self, mqtt: &AsyncClient) -> Result<(), String> {
        let base = self.base_topic();
        mqtt.subscribe(format!("{}/+/status", base), rumqttc::QoS::AtLeastOnce)
            .await
            .map_err(|e| e.to_string())?;
        mqtt.subscribe(format!("{}/+/command/ack", base), rumqttc::QoS::AtLeastOnce)
            .await
            .map_err(|e| e.to_string())?;
        mqtt.subscribe(format!("{}/+/online", base), rumqttc::QoS::AtLeastOnce)
            .await
            .map_err(|e| e.to_string())?;
        log::info!("UniLabOS adapter started for gateway: {}", self.gateway_id);
        Ok(())
    }

    async fn stop(&self) {
        log::info!("UniLabOS adapter stopped for gateway: {}", self.gateway_id);
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.values().cloned().collect()
    }

    fn parse_message(&self, topic: &str, payload: &[u8]) -> Option<OsdlEvent> {
        let base = self.base_topic();
        let suffix = topic.strip_prefix(&format!("{}/", base))?;

        // "{device_id}/status" | "{device_id}/command/ack" | "{device_id}/online"
        if let Some(device_id) = suffix.strip_suffix("/status") {
            let properties: HashMap<String, serde_json::Value> =
                serde_json::from_slice(payload).ok()?;
            Some(OsdlEvent::DeviceStatus(DeviceStatus {
                device_id: device_id.to_string(),
                timestamp: chrono_timestamp(),
                properties,
            }))
        } else if let Some(device_id) = suffix.strip_suffix("/command/ack") {
            let result: CommandResult = serde_json::from_slice(payload).ok()?;
            let _ = device_id;
            Some(OsdlEvent::CommandFeedback(result))
        } else if let Some(device_id) = suffix.strip_suffix("/online") {
            let online = payload == b"1";
            if online {
                // Device came online — emit with whatever we know from registry
                if let Some(dev) = self.devices.get(device_id) {
                    Some(OsdlEvent::DeviceOnline(dev.clone()))
                } else {
                    None
                }
            } else {
                Some(OsdlEvent::DeviceOffline {
                    device_id: device_id.to_string(),
                })
            }
        } else {
            None
        }
    }

    async fn dispatch_command(
        &self,
        mqtt: &AsyncClient,
        cmd: &DeviceCommand,
    ) -> Result<CommandResult, String> {
        let topic = format!("{}/{}/command", self.base_topic(), cmd.device_id);
        let payload = serde_json::to_vec(cmd).map_err(|e| e.to_string())?;
        mqtt.publish(topic, rumqttc::QoS::AtLeastOnce, false, payload)
            .await
            .map_err(|e| e.to_string())?;

        // Return Pending — actual result arrives async via command/ack topic
        Ok(CommandResult {
            command_id: cmd.command_id.clone(),
            device_id: cmd.device_id.clone(),
            status: CommandStatus::Pending,
            feedback: serde_json::Value::Null,
            result: None,
        })
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
