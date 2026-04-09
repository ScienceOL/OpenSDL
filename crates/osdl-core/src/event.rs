use crate::protocol::*;
use serde::Serialize;

/// Events emitted by the engine for the host application to consume.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum OsdlEvent {
    DeviceOnline(Device),
    DeviceOffline { device_id: String },
    DeviceStatus(DeviceStatus),
    CommandFeedback(CommandResult),
}
