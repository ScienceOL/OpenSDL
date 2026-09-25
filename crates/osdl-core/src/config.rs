use crate::media::{mediamtx::MediaGatewayConfig, MediaSourceConfig};
use crate::protocol::ActionSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsdlConfig {
    /// MQTT configuration. `None` disables the MQTT serial bridge entirely —
    /// engine runs without broker/subscriptions, and MQTT-backed features
    /// (node register/heartbeat, `handle_mqtt_message`) are inert.
    /// Use this when only ESP-NOW / direct-serial / TCP transports are needed.
    #[serde(default)]
    pub mqtt: Option<MqttConfig>,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
    /// ESP-NOW dongle boards plugged into this host (USB-CDC). Each entry
    /// owns one serial port and routes frames to/from its ESP-NOW nodes.
    #[serde(default)]
    pub espnow_dongles: Vec<EspNowDongleConfig>,
    /// Bus manifests: declares which devices hang off a single transport
    /// (shared RS-485 bus, etc.) when one node announces one hardware_id
    /// but physically bridges multiple addressed devices.
    ///
    /// When a node registers with `match_hardware_id`, the engine creates
    /// one `Device` per entry in `devices`, all sharing the node's
    /// transport. Without a matching bus entry, the legacy 1:1 behavior
    /// applies (one Device per REG).
    #[serde(default)]
    pub buses: Vec<BusConfig>,
    /// MAC → hardware_id table for ESP-NOW nodes that announce in the
    /// MAC-only REG form (no hardware_id baked into firmware). The engine
    /// looks up the announcing MAC here and proceeds through the same
    /// `buses` / 1:1 path as a legacy `REG <hardware_id>` would.
    ///
    /// MAC keys are uppercase hex without separators, e.g. `A4F00FD8555C`.
    /// Lets one firmware binary serve any station — identity is decided
    /// host-side, not at flash time.
    #[serde(default)]
    pub mac_assignments: HashMap<String, String>,
    /// Media sources (cameras, etc.). When non-empty the engine starts a
    /// mediamtx subprocess on `run()` to expose them via RTSP/HLS/WebRTC.
    #[serde(default)]
    pub media_sources: Vec<MediaSourceConfig>,
    /// Gateway config (ports, advertise host, mediamtx binary path). Only
    /// consulted when `media_sources` is non-empty.
    #[serde(default)]
    pub media_gateway: MediaGatewayConfig,
    /// Where snapshots and other transient camera artifacts get written.
    /// The CLI (`lab serve`) sets this from its `--data-dir` flag; tests
    /// that build configs directly can leave it `None` and the engine
    /// will fall back to the system temp directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    /// Optional local simulation world. Simulation devices use the same
    /// Device/Transport/ProtocolAdapter path as physical hardware, so an
    /// Agent and the UI can develop against them without a connected lab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation: Option<SimulationConfig>,
}

/// Configuration for a local simulation world.
///
/// `engine` is deliberately a string at this boundary. It is the runtime
/// capability negotiated by a future backend adapter (for example `rapier`,
/// `mujoco`, `isaac-sim`, or `gazebo`). The built-in `kinematic` backend is
/// deterministic and available in every OpenSDL build; unsupported engines
/// fail with an actionable error instead of silently falling back to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// Stable world identifier used in simulated transport/device ids.
    #[serde(default = "default_simulation_world_id")]
    pub world_id: String,
    /// Physics/runtime backend name. `kinematic` is the built-in backend.
    #[serde(default = "default_simulation_engine")]
    pub engine: String,
    /// Fixed update frequency for telemetry and deterministic stepping.
    #[serde(default = "default_simulation_tick_hz")]
    pub tick_hz: u32,
    /// Seed reserved for deterministic physics backends.
    #[serde(default)]
    pub seed: u64,
    /// Virtual devices exposed through the normal OpenSDL device contract.
    #[serde(default = "default_simulation_devices")]
    pub devices: Vec<SimulationDeviceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationDeviceConfig {
    /// Local id inside the simulation world, e.g. `heater-1`.
    pub id: String,
    pub device_type: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub actions: Vec<ActionSchema>,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
    /// Initial scene position in metres. The UI uses this to place entities.
    #[serde(default)]
    pub position: [f64; 3],
}

fn default_simulation_world_id() -> String {
    "lab-sim".into()
}

fn default_simulation_engine() -> String {
    "kinematic".into()
}

fn default_simulation_tick_hz() -> u32 {
    10
}

fn default_simulation_devices() -> Vec<SimulationDeviceConfig> {
    vec![
        SimulationDeviceConfig {
            id: "heater-1".into(),
            device_type: "simulation.heater".into(),
            role: Some("heater".into()),
            description: "Virtual temperature-controlled hotplate".into(),
            actions: vec![
                action_schema(
                    "set_temperature",
                    "Set the target temperature",
                    serde_json::json!({"type":"object","properties":{"temperature":{"type":"number","unit":"°C"}},"required":["temperature"]}),
                ),
                action_schema(
                    "start",
                    "Start heating",
                    serde_json::json!({"type":"object","properties":{}}),
                ),
                action_schema(
                    "stop",
                    "Stop heating",
                    serde_json::json!({"type":"object","properties":{}}),
                ),
            ],
            properties: HashMap::from([
                ("temperature".into(), serde_json::json!(22.0)),
                ("target_temperature".into(), serde_json::json!(22.0)),
                ("running".into(), serde_json::json!(false)),
            ]),
            position: [-1.6, 0.0, 0.0],
        },
        SimulationDeviceConfig {
            id: "stirrer-1".into(),
            device_type: "simulation.stirrer".into(),
            role: Some("stirrer".into()),
            description: "Virtual magnetic stirrer".into(),
            actions: vec![
                action_schema(
                    "set_speed",
                    "Set stirring speed",
                    serde_json::json!({"type":"object","properties":{"speed":{"type":"number","unit":"rpm"}},"required":["speed"]}),
                ),
                action_schema(
                    "start",
                    "Start stirring",
                    serde_json::json!({"type":"object","properties":{}}),
                ),
                action_schema(
                    "stop",
                    "Stop stirring",
                    serde_json::json!({"type":"object","properties":{}}),
                ),
            ],
            properties: HashMap::from([
                ("speed".into(), serde_json::json!(0.0)),
                ("running".into(), serde_json::json!(false)),
            ]),
            position: [0.0, 0.0, 0.0],
        },
        SimulationDeviceConfig {
            id: "valve-1".into(),
            device_type: "simulation.valve".into(),
            role: Some("valve".into()),
            description: "Virtual fluid control valve".into(),
            actions: vec![
                action_schema(
                    "open",
                    "Open the valve",
                    serde_json::json!({"type":"object","properties":{}}),
                ),
                action_schema(
                    "close",
                    "Close the valve",
                    serde_json::json!({"type":"object","properties":{}}),
                ),
            ],
            properties: HashMap::from([("state".into(), serde_json::json!("closed"))]),
            position: [1.6, 0.0, 0.0],
        },
        SimulationDeviceConfig {
            id: "probe-1".into(),
            device_type: "simulation.sensor".into(),
            role: Some("temperature_sensor".into()),
            description: "Virtual temperature probe".into(),
            actions: vec![action_schema(
                "read",
                "Read the current measurement",
                serde_json::json!({"type":"object","properties":{}}),
            )],
            properties: HashMap::from([
                ("temperature".into(), serde_json::json!(22.0)),
                ("unit".into(), serde_json::json!("°C")),
            ]),
            position: [0.0, 0.0, 1.8],
        },
    ]
}

fn action_schema(name: &str, description: &str, params: serde_json::Value) -> ActionSchema {
    ActionSchema {
        name: name.into(),
        description: description.into(),
        params,
    }
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            world_id: default_simulation_world_id(),
            engine: default_simulation_engine(),
            tick_hz: default_simulation_tick_hz(),
            seed: 0,
            devices: default_simulation_devices(),
        }
    }
}

impl SimulationConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.world_id.trim().is_empty() {
            return Err("simulation world_id must not be empty".into());
        }
        if self.tick_hz == 0 || self.tick_hz > 240 {
            return Err("simulation tick_hz must be between 1 and 240".into());
        }
        if self.engine.trim().is_empty() {
            return Err("simulation engine must not be empty".into());
        }
        if self.devices.is_empty() {
            return Err("simulation must define at least one device".into());
        }
        let mut ids = std::collections::HashSet::new();
        for device in &self.devices {
            if device.id.trim().is_empty() || !ids.insert(&device.id) {
                return Err(format!(
                    "simulation device id '{}' is empty or duplicated",
                    device.id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod simulation_tests {
    use super::SimulationConfig;

    #[test]
    fn default_simulation_is_valid_and_deterministic() {
        let config = SimulationConfig::default();
        config.validate().expect("default simulation config");
        assert_eq!(config.engine, "kinematic");
        assert_eq!(config.devices.len(), 4);
    }

    #[test]
    fn empty_engine_is_rejected() {
        let mut config = SimulationConfig::default();
        config.engine.clear();
        let error = config.validate().expect_err("an empty engine is invalid");
        assert!(error.contains("engine"));
    }
}

/// One physical bus (e.g., RS-485) reached through a single transport,
/// typically an ESP-NOW node. `match_hardware_id` is the ID the node
/// announces via REG; `devices` is the manifest of what the node bridges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusConfig {
    /// Node's announced hardware_id (must match `device_type` in one of
    /// the registry YAMLs — that's how REG matching works today).
    pub match_hardware_id: String,
    pub devices: Vec<BusDeviceConfig>,
}

/// One device on a shared bus. `device_type` picks the adapter/driver from
/// the registry; `local_id` becomes part of the engine Device's id so the
/// Agent can address each one independently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusDeviceConfig {
    /// Short id appended to the transport_id to form the final device_id,
    /// e.g. `pump-1` → `espnow:30EDA0B65B38:pump-1`.
    pub local_id: String,
    /// A device_type registered in one of the loaded adapter YAMLs.
    pub device_type: String,
    /// Optional semantic tag for the Agent: `stirrer`, `drain_valve`,
    /// `syringe_pump`, etc. Free-form; consumers should be lenient.
    #[serde(default)]
    pub role: Option<String>,
    /// Optional human/LLM-readable description override. Replaces the YAML
    /// default when present — useful for workflow-specific hints like
    /// "drain valve; 800 pulses = open".
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    #[serde(default = "default_mqtt_host")]
    pub host: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    #[serde(default = "default_keepalive")]
    pub keepalive_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EspNowDongleConfig {
    /// Serial device path of the dongle board, e.g. `/dev/cu.usbmodem*`
    /// (native USB on the Pocket-Dongle-S3) or `/dev/cu.usbserial-*` (older
    /// boards with an external USB-UART chip).
    pub port: String,
    #[serde(default = "default_espnow_baud")]
    pub baud_rate: u32,
}

fn default_espnow_baud() -> u32 {
    115200
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterConfig {
    /// Platform standard: "unilabos", "sila", etc.
    #[serde(rename = "type")]
    pub adapter_type: String,
    /// Path to local device registry directory for this adapter.
    #[serde(default)]
    pub registry_path: Option<String>,
}

fn default_mqtt_host() -> String {
    "localhost".into()
}
fn default_mqtt_port() -> u16 {
    1883
}
fn default_client_id() -> String {
    "osdl-mother".into()
}
fn default_keepalive() -> u64 {
    30
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: default_mqtt_host(),
            port: default_mqtt_port(),
            client_id: default_client_id(),
            keepalive_secs: default_keepalive(),
        }
    }
}
