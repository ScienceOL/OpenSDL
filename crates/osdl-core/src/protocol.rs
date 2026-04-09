use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A discovered device with its capabilities and current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub device_type: String,
    pub adapter: String,
    pub category: Vec<String>,
    pub description: String,
    pub online: bool,
    pub properties: HashMap<String, serde_json::Value>,
    pub actions: Vec<ActionSchema>,
}

/// Schema describing one executable action on a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSchema {
    pub name: String,
    pub description: String,
    pub goal_schema: serde_json::Value,
    pub feedback_schema: serde_json::Value,
    pub result_schema: serde_json::Value,
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
    pub goal: serde_json::Value,
}

/// Result of a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: String,
    pub device_id: String,
    pub status: CommandStatus,
    pub feedback: serde_json::Value,
    pub result: Option<serde_json::Value>,
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
