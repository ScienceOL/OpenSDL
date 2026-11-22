//! Updates to inventory reported by a configured transport owner.

use std::collections::HashMap;

use serde_json::Value;

use crate::{
    protocol::{Device, DeviceStatus},
    OsdlEvent,
};

use super::{now_millis, EngineHandle, READ_ONLY_ADAPTER};

impl EngineHandle {
    pub(super) async fn publish_device(&self, device: Device) -> Result<(), String> {
        let mut devices = self.devices.write().await;
        if devices.get(&device.id).is_some_and(|current| {
            current.transport_id != device.transport_id || current.adapter != device.adapter
        }) {
            return Err(format!(
                "device ID '{}' is already registered with another transport or probe",
                device.id
            ));
        }
        let event = device
            .online
            .then(|| OsdlEvent::DeviceOnline(device.clone()));
        devices.insert(device.id.clone(), device);
        drop(devices);
        if let Some(event) = event {
            self.emit(event);
        }
        Ok(())
    }

    pub async fn register_read_only_device(
        &self,
        id: &str,
        device_type: &str,
        description: &str,
    ) -> Result<(), String> {
        self.publish_device(Device {
            id: id.into(),
            transport_id: Self::probe_owner(id),
            device_type: device_type.into(),
            adapter: READ_ONLY_ADAPTER.into(),
            description: description.into(),
            online: false,
            properties: HashMap::new(),
            properties_timestamp_ms: 0,
            actions: Vec::new(),
            role: None,
        })
        .await?;
        self.emit(OsdlEvent::DeviceOffline {
            device_id: id.into(),
        });
        self.broadcast_status().await;
        Ok(())
    }

    pub async fn observe_read_only_device(
        &self,
        id: &str,
        properties: Option<HashMap<String, Value>>,
    ) {
        self.record_device_observation(id, &Self::probe_owner(id), properties)
            .await;
    }

    fn probe_owner(id: &str) -> String {
        format!("probe:{id}")
    }

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
