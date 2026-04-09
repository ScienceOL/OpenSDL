pub mod runze;
pub mod unilabos;

use crate::protocol::*;

/// Adapts a device driver ecosystem's registry format and serial protocol.
///
/// Each implementation understands one platform's way of describing devices
/// (e.g. UniLabOS YAML registry) and how to encode/decode serial bytes
/// for those devices. It does NOT touch MQTT directly — the engine handles that.
pub trait ProtocolAdapter: Send + Sync {
    /// Platform identifier, e.g. "unilabos", "sila".
    fn platform(&self) -> &str;

    /// Load device definitions from the local registry directory.
    fn load_registry(&mut self, path: &str) -> Result<(), String>;

    /// Given a hardware_id from a child node registration, try to match it
    /// to a known device type. Returns device metadata if matched.
    fn match_hardware(&self, hardware_id: &str) -> Option<DeviceMatch>;

    /// Encode a command into serial bytes to send to the device.
    fn encode_command(&self, device_type: &str, cmd: &DeviceCommand) -> Result<Vec<u8>, String>;

    /// Decode serial bytes received from a device into a status update.
    /// Returns None if the bytes are incomplete or not parseable.
    fn decode_response(
        &self,
        device_type: &str,
        bytes: &[u8],
    ) -> Option<std::collections::HashMap<String, serde_json::Value>>;
}

/// Result of matching a hardware_id to a known device type.
#[derive(Debug, Clone)]
pub struct DeviceMatch {
    pub device_type: String,
    pub description: String,
    pub actions: Vec<ActionSchema>,
}
