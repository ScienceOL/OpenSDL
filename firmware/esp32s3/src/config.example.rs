// Copy to src/config.rs and fill in real values. src/config.rs is gitignored.

pub const WIFI_SSID: &str = "YourWiFi";
pub const WIFI_PASSWORD: &str = "YourPassword";

pub const NODE_ID: &str = "pump-01";
pub const HARDWARE_ID: &str = "syringe_pump_with_valve.runze.SY03B-T06";

// MQTT_HOST = "" enables mDNS auto-discovery (_osdl._tcp.local).
// Set to an IP string to skip mDNS.
pub const MQTT_HOST: &str = "";
pub const MQTT_PORT: u16 = 1883;

pub const DEVICE_BAUD: u32 = 9600;
pub const UART_TX_PIN: i32 = 17;
pub const UART_RX_PIN: i32 = 18;
pub const RS485_DE_PIN: i32 = 16; // set -1 to disable RS-485 DE toggle

pub const HEARTBEAT_INTERVAL_MS: u64 = 10_000;
pub const MDNS_TIMEOUT_MS: u64 = 5_000;
pub const UART_SILENCE_MS: u64 = 50;
