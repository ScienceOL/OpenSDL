//! Protocol adapter for the built-in local simulation backend.
//!
//! Simulation deliberately uses the existing byte-oriented transport path:
//! the adapter serializes a small JSON action envelope and decodes a JSON
//! telemetry envelope. This keeps Lab Action Model calls identical for real
//! and virtual devices while leaving the physics/runtime implementation in
//! the transport/backend layer.

use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::protocol::DeviceCommand;
use serde_json::Value;
use std::collections::HashMap;

pub const PLATFORM: &str = "simulation";

#[derive(Debug, Default)]
pub struct SimulationAdapter;

impl SimulationAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ProtocolAdapter for SimulationAdapter {
    fn platform(&self) -> &str {
        PLATFORM
    }

    fn load_registry(&mut self, _path: &str) -> Result<(), String> {
        // Simulation devices are declared by the simulation world, not by a
        // physical registry directory.
        Ok(())
    }

    fn match_hardware(&self, _hardware_id: &str) -> Option<DeviceMatch> {
        None
    }

    fn encode_command(&self, _device_type: &str, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        serde_json::to_vec(cmd).map_err(|e| format!("simulation: encode command: {e}"))
    }

    fn decode_response(&self, _device_type: &str, bytes: &[u8]) -> Option<HashMap<String, Value>> {
        let envelope: Value = serde_json::from_slice(bytes).ok()?;
        let properties = envelope.get("properties")?.as_object()?;
        let mut decoded = properties
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<HashMap<_, _>>();
        decoded.insert("simulation".into(), Value::Bool(true));
        if let Some(engine) = envelope.get("engine") {
            decoded.insert("simulation_engine".into(), engine.clone());
        }
        if let Some(world_id) = envelope.get("world_id") {
            decoded.insert("simulation_world".into(), world_id.clone());
        }
        if let Some(action) = envelope.get("last_action") {
            decoded.insert("last_action".into(), action.clone());
        }
        Some(decoded)
    }
}
