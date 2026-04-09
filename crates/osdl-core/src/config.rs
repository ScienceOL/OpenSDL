use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsdlConfig {
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
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
pub struct AdapterConfig {
    /// Platform standard: "unilabos", "sila", etc.
    #[serde(rename = "type")]
    pub adapter_type: String,
    /// Identifier for this adapter instance.
    pub gateway_id: String,
    /// Path to local device registry directory for this adapter.
    #[serde(default)]
    pub registry_path: Option<String>,
}

fn default_mqtt_host() -> String { "localhost".into() }
fn default_mqtt_port() -> u16 { 1883 }
fn default_client_id() -> String { "osdl".into() }
fn default_keepalive() -> u64 { 30 }

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
