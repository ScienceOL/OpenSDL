//! Updates to inventory reported by a configured transport owner.

use std::collections::{hash_map::Entry, HashMap};

use serde_json::Value;

use crate::{
    media::{snapshot, v4l2::DepthCamera, MediaSourceConfig},
    protocol::{Device, DeviceStatus},
    OsdlEvent,
};

use super::{now_millis, EngineHandle, READ_ONLY_ADAPTER};

/// Writes probe samples to the device owner selected at registration.
pub struct ProbeObserver {
    engine: EngineHandle,
    device_id: String,
    owner: String,
}

impl ProbeObserver {
    pub async fn observe(&self, properties: Option<HashMap<String, Value>>) {
        self.engine
            .record_device_observation(&self.device_id, &self.owner, properties)
            .await;
    }
}

#[derive(Clone, Copy)]
enum Registration {
    Device,
    ColorCapture,
    DepthProbe,
}

impl EngineHandle {
    pub(super) async fn publish_device(&self, device: Device) -> Result<(), String> {
        self.publish_registration(device, Registration::Device)
            .await
    }

    pub(super) async fn publish_color_capture(&self, device: Device) -> Result<(), String> {
        self.publish_registration(device, Registration::ColorCapture)
            .await
    }

    async fn publish_registration(
        &self,
        mut device: Device,
        registration: Registration,
    ) -> Result<(), String> {
        let event = {
            let mut devices = self.devices.write().await;
            let current = match devices.entry(device.id.clone()) {
                Entry::Vacant(entry) => entry.insert(device),
                Entry::Occupied(entry) => {
                    let current = entry.into_mut();
                    let compatible = current.transport_id == device.transport_id
                        && match registration {
                            Registration::Device => current.adapter == device.adapter,
                            Registration::ColorCapture | Registration::DepthProbe => {
                                matches!(
                                    current.adapter.as_str(),
                                    READ_ONLY_ADAPTER | snapshot::PLATFORM
                                )
                            }
                        };
                    if !compatible {
                        return Err(format!(
                            "device ID '{}' is already registered with another transport or probe",
                            device.id
                        ));
                    }
                    match registration {
                        Registration::Device => *current = device,
                        // The probe owns identity, presence, and properties.
                        Registration::ColorCapture => {
                            current.adapter = device.adapter;
                            current.actions = device.actions;
                        }
                        Registration::DepthProbe => {
                            device.adapter = std::mem::take(&mut current.adapter);
                            device.actions = std::mem::take(&mut current.actions);
                            *current = device;
                        }
                    }
                    current
                }
            };
            current
                .online
                .then(|| OsdlEvent::DeviceOnline(current.clone()))
        };
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
        role: Option<&str>,
    ) -> Result<ProbeObserver, String> {
        self.register_probe(id, device_type, description, role, false)
            .await
    }

    pub async fn register_depth_camera_probe(
        &self,
        id: &str,
        device_type: &str,
        description: &str,
        probe: &DepthCamera,
    ) -> Result<ProbeObserver, String> {
        // Pair identical configured paths even while the device link is absent.
        let paired = self.config.media_sources.iter().any(|source| match source {
            MediaSourceConfig::V4l2Camera(camera) if camera.id == id => {
                probe.matches_color_device(&camera.device)
            }
            _ => false,
        });
        self.register_probe(id, device_type, description, Some("camera"), paired)
            .await
    }

    async fn register_probe(
        &self,
        id: &str,
        device_type: &str,
        description: &str,
        role: Option<&str>,
        paired: bool,
    ) -> Result<ProbeObserver, String> {
        let (owner, registration) = if paired {
            (snapshot::transport_id_for(id), Registration::DepthProbe)
        } else {
            (format!("probe:{id}"), Registration::Device)
        };
        self.publish_registration(
            Device {
                id: id.into(),
                transport_id: owner.clone(),
                device_type: device_type.into(),
                adapter: READ_ONLY_ADAPTER.into(),
                description: description.into(),
                online: false,
                properties: HashMap::new(),
                properties_timestamp_ms: 0,
                actions: Vec::new(),
                role: role.map(str::to_owned),
            },
            registration,
        )
        .await?;
        self.emit(OsdlEvent::DeviceOffline {
            device_id: id.into(),
        });
        self.broadcast_status().await;
        Ok(ProbeObserver {
            engine: self.clone(),
            device_id: id.into(),
            owner,
        })
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
