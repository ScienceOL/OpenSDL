use crate::media::{mediamtx::MediaGatewayConfig, MediaSourceConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
