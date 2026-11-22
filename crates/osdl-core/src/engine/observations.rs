//! Updates to inventory reported by a configured transport owner.

use std::collections::HashMap;

use serde_json::Value;

use crate::{protocol::DeviceStatus, OsdlEvent};

use super::{now_millis, EngineHandle};

impl EngineHandle {
    /// Update the owning transport's cached observation. Identical samples
    /// renew freshness without appending an unchanged event to history.
    pub(crate) async fn record_device_observation(
        &self,
        device_id: &str,
        transport_id: &str,
        properties: Option<HashMap<String, Value>>,
    ) {
        let event = {
            let mut devices = self.devices.write().await;
            let Some(device) = devices.get_mut(device_id) else {
                return;
            };
            if device.transport_id != transport_id {
                return;
            }
            match properties {
                Some(properties) => {
                    let timestamp = now_millis();
                    device.properties_timestamp_ms = timestamp;
                    let changed = device.properties != properties;
                    device.properties = properties;
                    if !device.online {
                        device.online = true;
                        Some(OsdlEvent::DeviceOnline(device.clone()))
                    } else if changed {
                        Some(OsdlEvent::DeviceStatus(DeviceStatus {
                            device_id: device_id.to_owned(),
                            timestamp,
                            properties: device.properties.clone(),
                        }))
                    } else {
                        None
                    }
                }
                None if device.online => {
                    device.online = false;
                    device.properties.clear();
                    device.properties_timestamp_ms = now_millis();
                    Some(OsdlEvent::DeviceOffline {
                        device_id: device_id.to_owned(),
                    })
                }
                None => None,
            }
        };
        if let Some(event) = event {
            self.emit(event);
        }
    }
}
