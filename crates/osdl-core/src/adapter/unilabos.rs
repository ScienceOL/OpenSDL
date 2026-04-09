use crate::adapter::runze::{self, RunzeConfig};
use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::protocol::*;
use std::collections::HashMap;

/// Adapter for the UniLabOS device description standard.
///
/// Reads UniLabOS YAML registry files to understand device capabilities,
/// maps hardware_ids to device types, and encodes/decodes serial protocols
/// (Modbus RTU, custom frames) for supported devices.
pub struct UniLabOsAdapter {
    /// hardware_id → device definition from registry YAML
    registry: HashMap<String, RegistryEntry>,
    /// device_type → RunzeConfig for Runze syringe pumps
    runze_configs: HashMap<String, RunzeConfig>,
}

#[derive(Debug, Clone)]
struct RegistryEntry {
    device_type: String,
    description: String,
    actions: Vec<ActionSchema>,
    // Future: protocol details (modbus address map, custom frame format, etc.)
}

impl UniLabOsAdapter {
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
            runze_configs: HashMap::new(),
        }
    }
}

impl Default for UniLabOsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolAdapter for UniLabOsAdapter {
    fn platform(&self) -> &str {
        "unilabos"
    }

    fn load_registry(&mut self, path: &str) -> Result<(), String> {
        log::info!("Loading UniLabOS registry from: {}", path);

        // Walk the registry directory for .yaml files
        let dir = std::fs::read_dir(path).map_err(|e| format!("read dir {}: {}", path, e))?;

        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
                continue;
            }

            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    Ok(yaml) => {
                        // A single YAML file may contain multiple devices
                        let entries = parse_registry_yaml(&yaml);
                        for (entry, runze_cfg) in entries {
                            log::info!(
                                "  Loaded device: {} ({})",
                                entry.device_type,
                                entry.description
                            );
                            let dt = entry.device_type.clone();
                            self.registry.insert(dt.clone(), entry);
                            if let Some(cfg) = runze_cfg {
                                log::info!("    → Runze driver configured");
                                self.runze_configs.insert(dt, cfg);
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
            "UniLabOS registry loaded: {} device types",
            self.registry.len()
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
        if let Some(config) = self.runze_configs.get(device_type) {
            return runze::encode(config, cmd);
        }
        Err(format!(
            "encode_command not yet implemented for device type: {}",
            device_type
        ))
    }

    fn decode_response(
        &self,
        device_type: &str,
        bytes: &[u8],
    ) -> Option<HashMap<String, serde_json::Value>> {
        if let Some(config) = self.runze_configs.get(device_type) {
            return runze::decode(config, bytes);
        }
        None
    }
}

/// Parse a UniLabOS registry YAML into (RegistryEntry, Option<RunzeConfig>) tuples.
///
/// A YAML file may define a single device (has `device_type` at root) or
/// multiple devices (has `devices` list). Each device that has
/// `driver: "runze"` in its metadata gets a RunzeConfig.
fn parse_registry_yaml(yaml: &serde_yaml::Value) -> Vec<(RegistryEntry, Option<RunzeConfig>)> {
    // Multi-device YAML: top-level `devices` array
    if let Some(devices) = yaml.get("devices").and_then(|v| v.as_sequence()) {
        return devices.iter().filter_map(|d| parse_single_device(d)).collect();
    }
    // Single-device YAML: root is the device
    if let Some(pair) = parse_single_device(yaml) {
        return vec![pair];
    }
    vec![]
}

fn parse_single_device(yaml: &serde_yaml::Value) -> Option<(RegistryEntry, Option<RunzeConfig>)> {
    let device_type = yaml.get("device_type")?.as_str()?.to_string();
    let description = yaml
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut actions = Vec::new();

    if let Some(mappings) = yaml.get("action_value_mappings").and_then(|v| v.as_sequence()) {
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

    // Detect Runze driver from YAML metadata
    let runze_cfg = if yaml
        .get("driver")
        .and_then(|v| v.as_str())
        .map(|s| s == "runze")
        .unwrap_or(false)
    {
        let mut cfg = RunzeConfig::default();
        if let Some(addr) = yaml.get("address").and_then(|v| v.as_str()) {
            cfg.address = addr.to_string();
        } else if let Some(addr) = yaml.get("address").and_then(|v| v.as_u64()) {
            cfg.address = addr.to_string();
        }
        if let Some(vol) = yaml.get("max_volume").and_then(|v| v.as_f64()) {
            cfg.max_volume = vol;
        }
        if let Some(steps) = yaml.get("total_steps").and_then(|v| v.as_u64()) {
            cfg.total_steps = steps as u32;
        }
        if let Some(steps) = yaml.get("total_steps_vel").and_then(|v| v.as_u64()) {
            cfg.total_steps_vel = steps as u32;
        }
        Some(cfg)
    } else {
        None
    };

    Some((entry, runze_cfg))
}
