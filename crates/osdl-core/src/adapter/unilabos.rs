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
                        if let Some(entry) = parse_registry_yaml(&yaml) {
                            log::info!(
                                "  Loaded device: {} ({})",
                                entry.device_type,
                                entry.description
                            );
                            // Use device_type as hardware_id for now.
                            // Real mapping would come from the YAML's hardware_id field.
                            self.registry
                                .insert(entry.device_type.clone(), entry);
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
        // TODO: look up protocol spec for device_type, encode serial bytes
        // For now, return a placeholder that demonstrates the flow
        let _ = device_type;
        log::debug!(
            "Encoding command {} for device {}",
            cmd.action,
            cmd.device_id
        );
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
        // TODO: look up protocol spec for device_type, decode serial bytes
        let _ = (device_type, bytes);
        None
    }
}

/// Parse a UniLabOS registry YAML into a RegistryEntry.
fn parse_registry_yaml(yaml: &serde_yaml::Value) -> Option<RegistryEntry> {
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

    Some(RegistryEntry {
        device_type,
        description,
        actions,
    })
}
