use crate::protocol::*;
use serde::Serialize;

/// Events emitted by the engine for the host application to consume.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum OsdlEvent {
    /// A child node registered and was matched to a device driver.
    DeviceOnline(Device),
    /// A child node went offline (LWT or heartbeat timeout).
    DeviceOffline { device_id: String },
    /// Device reported new status (parsed from serial response).
    DeviceStatus(DeviceStatus),
    /// A command completed (success or failure).
    CommandResult(CommandResult),
    /// A child node registered but no matching driver was found.
    UnknownNode {
        node_id: String,
        hardware_id: String,
    },
}
