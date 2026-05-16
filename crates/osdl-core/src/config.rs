use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsdlConfig {
    /// MQTT configuration. `None` disables the MQTT serial bridge entirely —
    /// engine runs without broker/subscriptions, and MQTT-backed features
    /// (child node register/heartbeat, `handle_mqtt_message`) are inert.
    /// Use this when only ESP-NOW / direct-serial / TCP transports are needed.
    #[serde(default)]
    pub mqtt: Option<MqttConfig>,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
    /// ESP-NOW gateway boards plugged into this host (USB-CDC). Each entry
    /// owns one serial port and routes frames to/from its ESP-NOW children.
    #[serde(default)]
    pub espnow_gateways: Vec<EspNowGatewayConfig>,
    /// Bus manifests: declares which devices hang off a single transport
    /// (shared RS-485 bus, etc.) when one child announces one hardware_id
    /// but physically bridges multiple addressed devices.
    ///
    /// When a child registers with `match_hardware_id`, the engine creates
    /// one `Device` per entry in `devices`, all sharing the child's
    /// transport. Without a matching bus entry, the legacy 1:1 behavior
    /// applies (one Device per REG).
    #[serde(default)]
    pub buses: Vec<BusConfig>,
}

/// One physical bus (e.g., RS-485) reached through a single transport,
/// typically an ESP-NOW child. `match_hardware_id` is the ID the child
/// announces via REG; `devices` is the manifest of what the child bridges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusConfig {
    /// Child's announced hardware_id (must match `device_type` in one of
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
pub struct EspNowGatewayConfig {
    /// Serial device path of the gateway board, e.g. `/dev/cu.usbserial-A5069RR4`.
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
