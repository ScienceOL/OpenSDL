pub mod unilabos;

use crate::event::OsdlEvent;
use crate::protocol::*;
use async_trait::async_trait;
use rumqttc::AsyncClient;

/// Adapts a device description standard + MQTT communication protocol.
///
/// Each implementation understands one platform's way of describing devices
/// (e.g. UniLabOS YAML registry) and its MQTT topic/payload conventions.
/// It does NOT require the platform software to be running.
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform identifier, e.g. "unilabos", "sila".
    fn platform(&self) -> &str;

    /// Load device definitions from the local registry.
    fn load_registry(&mut self, path: &str) -> Result<(), String>;

    /// Subscribe to relevant MQTT topics for this platform.
    async fn start(&self, mqtt: &AsyncClient) -> Result<(), String>;

    /// Stop and clean up.
    async fn stop(&self);

    /// Return all devices known from the loaded registry.
    fn devices(&self) -> Vec<Device>;

    /// Parse an incoming MQTT message into an OsdlEvent, if it belongs to this adapter.
    fn parse_message(&self, topic: &str, payload: &[u8]) -> Option<OsdlEvent>;

    /// Serialize and publish a command via MQTT in this platform's format.
    async fn dispatch_command(
        &self,
        mqtt: &AsyncClient,
        cmd: &DeviceCommand,
    ) -> Result<CommandResult, String>;
}
