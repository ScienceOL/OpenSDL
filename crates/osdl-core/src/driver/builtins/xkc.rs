//! XKC non-contact liquid level sensor codec (Modbus RTU).
//!
//! The sensor reports liquid presence via RSSI signal strength.
//! Level detection: rssi > threshold means liquid is present.
//!
//! Used by ChinWe workstation (sensor ID 6) over TCP.

use crate::driver::util::modbus_rtu;
use crate::protocol::DeviceCommand;
use std::collections::HashMap;

/// Device-specific configuration.
#[derive(Debug, Clone)]
pub struct XkcConfig {
    /// Modbus slave ID (typically 6).
    pub slave_id: u8,
    /// RSSI threshold for liquid detection (default 300).
    pub threshold: u16,
}

impl Default for XkcConfig {
    fn default() -> Self {
        Self {
            slave_id: 6,
            threshold: 300,
        }
    }
}

/// Encode a high-level command into Modbus RTU bytes for the XKC sensor.
pub fn encode(config: &XkcConfig, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
    match cmd.action.as_str() {
        "read_level" => {
            // Read 2 registers starting at address 0x0001
            Ok(modbus_rtu::build_read_registers(
                config.slave_id,
                0x0001,
                0x0002,
            ))
        }
        other => Err(format!("unknown xkc action: {}", other)),
    }
}

/// Decode a Modbus RTU response from the XKC sensor.
///
/// Returns `None` if the response is not from this slave or is invalid.
pub fn decode(config: &XkcConfig, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
    let (slave, func, payload) = modbus_rtu::parse_response(bytes)?;

    if slave != config.slave_id {
        return None;
    }
    if func != 0x03 {
        return None;
    }

    let registers = modbus_rtu::parse_read_registers(&payload)?;

    // Extract RSSI value. The sensor returns 2 registers;
    // RSSI is typically in the second register (or combined).
    let rssi = if registers.len() >= 2 {
        registers[1]
    } else if !registers.is_empty() {
        registers[0]
    } else {
        return None;
    };

    let level = rssi > config.threshold;

    let mut props = HashMap::new();
    props.insert("rssi".into(), serde_json::json!(rssi));
    props.insert("level".into(), serde_json::json!(level));
    Some(props)
}

// --------------- Driver trait impl ---------------

use crate::driver::Driver;
use crate::driver::registry::DriverRegistry;

/// XKC liquid level sensor driver instance (pre-configured).
pub struct XkcDriver {
    config: XkcConfig,
}

impl Driver for XkcDriver {
    fn name(&self) -> &str {
        "xkc"
    }

    fn encode(&self, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        encode(&self.config, cmd)
    }

    fn decode(&self, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
        decode(&self.config, bytes)
    }
}

/// Create an XkcDriver from YAML device config.
pub fn create_from_yaml(yaml: &serde_yaml::Value) -> Result<Box<dyn Driver>, String> {
    let mut cfg = XkcConfig::default();
    if let Some(id) = yaml.get("slave_id").and_then(|v| v.as_u64()) {
        cfg.slave_id = id as u8;
    }
    if let Some(t) = yaml.get("threshold").and_then(|v| v.as_u64()) {
        cfg.threshold = t as u16;
    }
    Ok(Box::new(XkcDriver { config: cfg }))
}

/// Register this driver with the registry.
pub fn register(registry: &mut DriverRegistry) {
    registry.register("xkc", create_from_yaml);
}

// --------------- Tests ---------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DeviceCommand;

    fn default_config() -> XkcConfig {
        XkcConfig::default()
    }

    fn make_cmd(action: &str, params: serde_json::Value) -> DeviceCommand {
        DeviceCommand {
            command_id: "test".into(),
            device_id: "sensor-06".into(),
            action: action.into(),
            params,
        }
    }

    #[test]
    fn test_encode_read_level() {
        let config = default_config();
        let cmd = make_cmd("read_level", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes[0], 6); // slave
        assert_eq!(bytes[1], 0x03); // read regs
        assert_eq!(bytes[2], 0x00);
        assert_eq!(bytes[3], 0x01); // start addr
        assert_eq!(bytes[4], 0x00);
        assert_eq!(bytes[5], 0x02); // count
        assert_eq!(bytes.len(), 8);
    }

    #[test]
    fn test_encode_unknown_action() {
        let config = default_config();
        let cmd = make_cmd("calibrate", serde_json::json!({}));
        assert!(encode(&config, &cmd).is_err());
    }

    #[test]
    fn test_decode_level_above_threshold() {
        let config = default_config(); // threshold = 300
        // Response: slave=6, fn=0x03, 2 registers: reg0=0, reg1=500 (above 300)
        let mut frame = vec![0x06, 0x03, 0x04, 0x00, 0x00, 0x01, 0xF4];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);

        let props = decode(&config, &frame).unwrap();
        assert_eq!(props["rssi"], 500);
        assert_eq!(props["level"], true);
    }

    #[test]
    fn test_decode_level_below_threshold() {
        let config = default_config(); // threshold = 300
        // Response: slave=6, fn=0x03, 2 registers: reg0=0, reg1=100 (below 300)
        let mut frame = vec![0x06, 0x03, 0x04, 0x00, 0x00, 0x00, 0x64];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);

        let props = decode(&config, &frame).unwrap();
        assert_eq!(props["rssi"], 100);
        assert_eq!(props["level"], false);
    }

    #[test]
    fn test_decode_wrong_slave_returns_none() {
        let config = default_config(); // slave_id = 6
        // Response from slave 1
        let mut frame = vec![0x01, 0x03, 0x04, 0x00, 0x00, 0x01, 0xF4];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);

        assert!(decode(&config, &frame).is_none());
    }

    #[test]
    fn test_decode_wrong_function_returns_none() {
        let config = default_config();
        // Write ack (fn=0x06) instead of read response
        let mut frame = vec![0x06, 0x06, 0x00, 0x01, 0x00, 0x01];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);

        assert!(decode(&config, &frame).is_none());
    }
}
