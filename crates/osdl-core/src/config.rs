use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsdlConfig {
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
    /// ESP-NOW gateway boards plugged into this host (USB-CDC). Each entry
    /// owns one serial port and routes frames to/from its ESP-NOW children.
    #[serde(default)]
    pub espnow_gateways: Vec<EspNowGatewayConfig>,
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
