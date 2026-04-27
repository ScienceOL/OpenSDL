use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::driver::registry::DriverRegistry;
use crate::driver::Driver;
use crate::protocol::*;
use std::collections::HashMap;

/// Adapter for the UniLabOS device description standard.
///
/// Reads UniLabOS YAML registry files, creates `Driver` instances via the
/// `DriverRegistry`, and dispatches encode/decode through them.
///
/// The adapter itself contains no driver-specific code — all protocol
/// knowledge lives in the driver modules under `drivers/`.
pub struct UniLabOsAdapter {
    /// device_type → device metadata from registry YAML
    registry: HashMap<String, RegistryEntry>,
    /// device_type → configured Driver instance
    drivers: HashMap<String, Box<dyn Driver>>,
    /// Factory registry for creating drivers by name
    driver_registry: DriverRegistry,
}

#[derive(Debug, Clone)]
struct RegistryEntry {
    device_type: String,
    description: String,
    actions: Vec<ActionSchema>,
}

impl UniLabOsAdapter {
    pub fn new(driver_registry: DriverRegistry) -> Self {
        Self {
            registry: HashMap::new(),
            drivers: HashMap::new(),
            driver_registry,
        }
    }
}

impl ProtocolAdapter for UniLabOsAdapter {
    fn platform(&self) -> &str {
        "unilabos"
    }

    fn load_registry(&mut self, path: &str) -> Result<(), String> {
        log::info!("Loading UniLabOS registry from: {}", path);

        let dir = std::fs::read_dir(path).map_err(|e| format!("read dir {}: {}", path, e))?;

        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                continue;
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    Ok(yaml) => {
                        let device_nodes = collect_device_nodes(&yaml);
                        for device_yaml in device_nodes {
                            if let Some((entry, driver_name)) =
                                parse_registry_entry(&device_yaml)
                            {
                                log::info!(
                                    "  Loaded device: {} ({})",
                                    entry.device_type,
                                    entry.description
                                );
                                let dt = entry.device_type.clone();
                                self.registry.insert(dt.clone(), entry);

                                if let Some(name) = driver_name {
                                    match self.driver_registry.create(&name, &device_yaml) {
                                        Ok(driver) => {
                                            log::info!(
                                                "    → {} driver configured",
                                                driver.name()
                                            );
                                            self.drivers.insert(dt, driver);
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "    Failed to create driver '{}': {}",
                                                name,
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("  Failed to parse {}: {}", path.display(), e);
                    }
                },
                Err(e) => {
                    log::warn!("  Failed to read {}: {}", path.display(), e);
                }
            }
        }

        log::info!(
            "UniLabOS registry loaded: {} device types, {} drivers",
            self.registry.len(),
            self.drivers.len()
        );
        Ok(())
    }

    fn match_hardware(&self, hardware_id: &str) -> Option<DeviceMatch> {
        let entry = self.registry.get(hardware_id)?;
        Some(DeviceMatch {
            device_type: entry.device_type.clone(),
            description: entry.description.clone(),
            actions: entry.actions.clone(),
        })
    }

    fn encode_command(&self, device_type: &str, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        self.drivers
            .get(device_type)
            .ok_or_else(|| format!("no driver for device type: {}", device_type))?
            .encode(cmd)
    }

    fn decode_response(
        &self,
        device_type: &str,
        bytes: &[u8],
    ) -> Option<HashMap<String, serde_json::Value>> {
        self.drivers.get(device_type)?.decode(bytes)
    }
}

/// Collect device YAML nodes from a registry file.
///
/// Supports both multi-device format (`devices:` list) and
/// single-device format (root is the device).
fn collect_device_nodes(yaml: &serde_yaml::Value) -> Vec<serde_yaml::Value> {
    if let Some(devices) = yaml.get("devices").and_then(|v| v.as_sequence()) {
        return devices.clone();
    }
    // Single-device: root is the device node
    if yaml.get("device_type").is_some() {
        return vec![yaml.clone()];
    }
    vec![]
}

/// Parse a device YAML node into registry metadata + optional driver name.
///
/// This function only extracts framework-level fields (device_type,
/// description, actions). Driver-specific config (address, slave_id, etc.)
/// is handled by each driver's `create_from_yaml` factory.
fn parse_registry_entry(
    yaml: &serde_yaml::Value,
) -> Option<(RegistryEntry, Option<String>)> {
    let device_type = yaml.get("device_type")?.as_str()?.to_string();
    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let driver_name = yaml
        .get("driver")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut actions = Vec::new();
    if let Some(mappings) = yaml
        .get("action_value_mappings")
        .and_then(|v| v.as_sequence())
    {
        for mapping in mappings {
            if let Some(name) = mapping.get("action_name").and_then(|v| v.as_str()) {
                let desc = mapping
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = mapping
                    .get("goal_schema")
                    .map(|v| serde_json::to_value(v).unwrap_or_default())
                    .unwrap_or_default();
                actions.push(ActionSchema {
                    name: name.to_string(),
                    description: desc,
                    params,
                });
            }
        }
    }

    let entry = RegistryEntry {
        device_type,
        description,
        actions,
    };
    Some((entry, driver_name))
}
