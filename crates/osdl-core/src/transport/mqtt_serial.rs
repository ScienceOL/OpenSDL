//! MQTT Serial transport — bytes tunneled over MQTT to/from an ESP32 bridge node.
//!
//! This is the original transport for OpenSDL: an ESP32 child node acts as a
//! transparent serial-to-MQTT bridge. The mother node sends bytes to
//! `osdl/serial/{node_id}/tx` and receives bytes from `osdl/serial/{node_id}/rx`.

use super::Transport;
use async_trait::async_trait;
use rumqttc::AsyncClient;

/// Transport that tunnels serial bytes over MQTT topics.
pub struct MqttSerialTransport {
    node_id: String,
    client: AsyncClient,
}

impl MqttSerialTransport {
    pub fn new(node_id: String, client: AsyncClient) -> Self {
        Self { node_id, client }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

#[async_trait]
impl Transport for MqttSerialTransport {
    fn transport_type(&self) -> &str {
        "mqtt_serial"
    }

    fn description(&self) -> String {
        format!("MQTT serial bridge via node {}", self.node_id)
    }

    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let topic = format!("osdl/serial/{}/tx", self.node_id);
        self.client
            .publish(topic, rumqttc::QoS::AtLeastOnce, false, bytes)
            .await
            .map_err(|e| e.to_string())
    }

    fn is_connected(&self) -> bool {
        true // MQTT client handles reconnection internally
    }
}
