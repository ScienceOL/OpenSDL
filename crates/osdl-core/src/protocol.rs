use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// === Child Node (MQTT serial bridge) ===

/// A child node (ESP32 serial bridge) connected via MQTT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub node_id: String,
    pub hardware_id: String,
    pub baud_rate: u32,
    pub online: bool,
    /// Device ID assigned after driver match (None if unrecognized hardware).
    pub device_id: Option<String>,
}

/// Registration payload published by child node on boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistration {
    pub hardware_id: String,
    #[serde(default = "default_baud")]
    pub baud_rate: u32,
}

fn default_baud() -> u32 {
    9600
}

// === Device ===

/// A discovered device with its capabilities and current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    /// Transport identifier — how to reach this device.
    /// For MQTT serial: the node_id (e.g., "pump-01")
    /// For direct serial: the port path (e.g., "/dev/ttyUSB0")
    /// For TCP: the host:port (e.g., "192.168.1.50:502")
    pub transport_id: String,
    pub device_type: String,
    pub adapter: String,
    pub description: String,
    pub online: bool,
    pub properties: HashMap<String, serde_json::Value>,
    pub actions: Vec<ActionSchema>,
    /// Optional semantic tag (e.g. "stirrer", "drain_valve", "syringe_pump")
    /// sourced from the `buses:` config. Free-form; consumers should treat
    /// unknown values as no-op. Surfaced to the Agent via `list_devices`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Schema describing one executable action on a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSchema {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Real-time status update from a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub device_id: String,
    pub timestamp: i64,
    pub properties: HashMap<String, serde_json::Value>,
}

/// A command to send to a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCommand {
    pub command_id: String,
    pub device_id: String,
    pub action: String,
    pub params: serde_json::Value,
}

/// Result of a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: String,
    pub device_id: String,
    pub status: CommandStatus,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
