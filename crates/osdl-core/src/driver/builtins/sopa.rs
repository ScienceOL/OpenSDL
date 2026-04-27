//! SOPA pneumatic pipette codec (ASCII text protocol over RS-485).
//!
//! Frame format:
//!   TX: header('/' or '[') + address + command_body + 'E' + checksum(1 byte)
//!   RX: header + address + data(variable) + 'E' + checksum(1 byte)
//!
//! The checksum is the low byte of the sum of all preceding bytes.
//!
//! Shares the same RS-485 bus as XYZ stepper motors (Modbus RTU),
//! but uses a different protocol (ASCII text vs binary Modbus).

use crate::protocol::DeviceCommand;
use std::collections::HashMap;

/// Device-specific configuration.
#[derive(Debug, Clone)]
pub struct SopaConfig {
    /// Device address on the RS-485 bus (typically 4).
    pub address: u8,
    /// Header character: '/' for terminal/debug, '[' for OEM.
    pub comm_type: char,
}

impl Default for SopaConfig {
    fn default() -> Self {
        Self {
            address: 4,
            comm_type: '/',
        }
    }
}

/// Build a complete SOPA command frame with header, address, body, tail, and checksum.
fn build_command(config: &SopaConfig, cmd_body: &str) -> Vec<u8> {
    let cmd_str = format!("/{}{cmd_body}E", config.address);
    let cmd_bytes = cmd_str.as_bytes();
    let checksum: u8 = cmd_bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    let mut result = cmd_bytes.to_vec();
    result.push(checksum);
    result
}

/// Encode a high-level command into SOPA protocol bytes.
pub fn encode(config: &SopaConfig, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
    match cmd.action.as_str() {
        "initialize" => Ok(build_command(config, "HE")),

        "eject_tip" => Ok(build_command(config, "RE")),

        "aspirate" => {
            let volume = cmd
                .params
                .get("volume")
                .and_then(|v| v.as_f64())
                .ok_or("aspirate requires 'volume' (float, uL)")?;
            Ok(build_command(config, &format!("P{}", volume as i64)))
        }

        "dispense" => {
            let volume = cmd
                .params
                .get("volume")
                .and_then(|v| v.as_f64())
                .ok_or("dispense requires 'volume' (float, uL)")?;
            Ok(build_command(config, &format!("D{}", volume as i64)))
        }

        "move_absolute" => {
            let position = cmd
                .params
                .get("position")
                .and_then(|v| v.as_f64())
                .ok_or("move_absolute requires 'position' (float, uL)")?;
            Ok(build_command(config, &format!("A{}", position as i64)))
        }

        "query_status" => Ok(build_command(config, "Q")),
        "query_position" => Ok(build_command(config, "Q18")),
        "query_tip" => Ok(build_command(config, "Q28")),
        "get_firmware" => Ok(build_command(config, "VE")),

        "set_max_speed" => {
            let speed = cmd
                .params
                .get("speed")
                .and_then(|v| v.as_u64())
                .ok_or("set_max_speed requires 'speed' (int)")?;
            Ok(build_command(config, &format!("s{}", speed)))
        }

        "set_start_speed" => {
            let speed = cmd
                .params
                .get("speed")
                .and_then(|v| v.as_u64())
                .ok_or("set_start_speed requires 'speed' (int)")?;
            Ok(build_command(config, &format!("b{}", speed)))
        }

        "set_cutoff_speed" => {
            let speed = cmd
                .params
                .get("speed")
                .and_then(|v| v.as_u64())
                .ok_or("set_cutoff_speed requires 'speed' (int)")?;
            Ok(build_command(config, &format!("c{}", speed)))
        }

        "set_acceleration" => {
            let accel = cmd
                .params
                .get("acceleration")
                .and_then(|v| v.as_u64())
                .ok_or("set_acceleration requires 'acceleration' (int)")?;
            Ok(build_command(config, &format!("a{}", accel)))
        }

        other => Err(format!("unknown sopa action: {}", other)),
    }
}

/// Decode a SOPA response into status properties.
///
/// Returns `None` if the response is not addressed to this device
/// or is unparseable.
///
/// Hardware returns binary address bytes (e.g., 0x04 for address 4),
/// not ASCII digits. Multiple frames may be concatenated in a single
/// TCP/serial chunk — we find the LAST frame matching our address
/// since it typically carries the final answer (e.g., tip query result).
///
/// Frame layout: `/ <addr_byte> <data...> E <checksum_byte>`
pub fn decode(config: &SopaConfig, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
    // Find all '/' positions — each starts a potential frame.
    let slash_positions: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|(_, &b)| b == b'/')
        .map(|(i, _)| i)
        .collect();

    if slash_positions.is_empty() {
        return None;
    }

    // Collect frames addressed to us (match binary addr OR ASCII addr).
    let binary_addr = config.address;
    let ascii_addr = b'0' + config.address; // e.g., address 4 → '4' (0x34)

    let mut matched_frames: Vec<&[u8]> = Vec::new();

    for (idx, &start) in slash_positions.iter().enumerate() {
        // Frame must have at least: '/' + addr + 1 data byte + 'E' + checksum = 5 bytes
        if start + 4 >= bytes.len() {
            continue;
        }

        let addr_byte = bytes[start + 1];
        if addr_byte != binary_addr && addr_byte != ascii_addr {
            continue;
        }

        // Frame extends to the next '/' or end of buffer.
        let end = if idx + 1 < slash_positions.len() {
            slash_positions[idx + 1]
        } else {
            bytes.len()
        };

        matched_frames.push(&bytes[start..end]);
    }

    // Use the last matching frame (most recent/complete answer).
    let frame = matched_frames.last()?;

    // Data region: everything after '/' + addr_byte, i.e., frame[2..]
    // The tail is 'E' + checksum (2 bytes), but 'E' may also appear in data.
    // Find the LAST 'E' (0x45) in the frame — it's the tail marker.
    let data_region = &frame[2..];

    let mut props = HashMap::new();

    // Scan for tip status: 'T' followed by '1' or '0'
    for window in data_region.windows(2) {
        if window[0] == b'T' && window[1] == b'1' {
            props.insert("tip_present".into(), serde_json::json!(true));
            break;
        } else if window[0] == b'T' && window[1] == b'0' {
            props.insert("tip_present".into(), serde_json::json!(false));
            break;
        }
    }

    // Parse status/error code: first byte of data_region.
    // Binary codes 0x00-0x0F map to error states, ASCII hex digits also accepted.
    if !data_region.is_empty() {
        let code_byte = data_region[0];
        let code = if code_byte <= 0x0F {
            // Binary status code
            Some(code_byte as u32)
        } else if code_byte.is_ascii_hexdigit() {
            // ASCII hex digit (e.g., '0'=0x30 → 0)
            (code_byte as char).to_digit(16)
        } else {
            None
        };

        if let Some(c) = code {
            let status_str = match c {
                0 => "no_error",
                1 => "action_incomplete",
                2 => "not_initialized",
                3 => "overload",
                4 => "invalid_command",
                5 => "lld_fault",
                6 => "not_initialized",
                0xA => "plunger_overload",
                0xD => "air_aspirate",
                0xE => "needle_block",
                _ => "unknown",
            };
            props.insert("status".into(), serde_json::json!(status_str));
        }
    }

    if props.is_empty() {
        props.insert("status".into(), serde_json::json!("ok"));
    }

    Some(props)
}

// --------------- Driver trait impl ---------------

use crate::driver::Driver;
use crate::driver::registry::DriverRegistry;

/// SOPA pipette driver instance (pre-configured).
pub struct SopaDriver {
    config: SopaConfig,
}

impl Driver for SopaDriver {
    fn name(&self) -> &str {
        "sopa"
    }

    fn encode(&self, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        encode(&self.config, cmd)
    }

    fn decode(&self, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
        decode(&self.config, bytes)
    }
}

/// Create a SopaDriver from YAML device config.
pub fn create_from_yaml(yaml: &serde_yaml::Value) -> Result<Box<dyn Driver>, String> {
    let mut cfg = SopaConfig::default();
    if let Some(addr) = yaml.get("address").and_then(|v| v.as_u64()) {
        cfg.address = addr as u8;
    }
    if let Some(ct) = yaml
        .get("comm_type")
        .and_then(|v| v.as_str())
        .and_then(|s| s.chars().next())
    {
        cfg.comm_type = ct;
    }
    Ok(Box::new(SopaDriver { config: cfg }))
}

/// Register this driver with the registry.
pub fn register(registry: &mut DriverRegistry) {
    registry.register("sopa", create_from_yaml);
}

// --------------- Tests ---------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DeviceCommand;

    fn default_config() -> SopaConfig {
        SopaConfig::default()
    }

    fn make_cmd(action: &str, params: serde_json::Value) -> DeviceCommand {
        DeviceCommand {
            command_id: "test".into(),
            device_id: "pipette-01".into(),
            action: action.into(),
            params,
        }
    }

    #[test]
    fn test_build_command_checksum() {
        let config = default_config();
        let frame = build_command(&config, "HE");
        // Frame should be: "/4HEE" + checksum
        let expected_prefix = b"/4HEE";
        assert_eq!(&frame[..5], expected_prefix);
        let checksum: u8 = expected_prefix.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        assert_eq!(frame[5], checksum);
        assert_eq!(frame.len(), 6);
    }

    #[test]
    fn test_encode_initialize() {
        let config = default_config();
        let cmd = make_cmd("initialize", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert!(bytes.starts_with(b"/4HEE"));
    }

    #[test]
    fn test_encode_aspirate() {
        let config = default_config();
        let cmd = make_cmd("aspirate", serde_json::json!({"volume": 200.0}));
        let bytes = encode(&config, &cmd).unwrap();
        assert!(bytes.starts_with(b"/4P200E"));
    }

    #[test]
    fn test_encode_dispense() {
        let config = default_config();
        let cmd = make_cmd("dispense", serde_json::json!({"volume": 100.0}));
        let bytes = encode(&config, &cmd).unwrap();
        assert!(bytes.starts_with(b"/4D100E"));
    }

    #[test]
    fn test_encode_eject_tip() {
        let config = default_config();
        let cmd = make_cmd("eject_tip", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert!(bytes.starts_with(b"/4REE"));
    }

    #[test]
    fn test_encode_query_tip() {
        let config = default_config();
        let cmd = make_cmd("query_tip", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert!(bytes.starts_with(b"/4Q28E"));
    }

    #[test]
    fn test_encode_unknown_action() {
        let config = default_config();
        let cmd = make_cmd("fly", serde_json::json!({}));
        assert!(encode(&config, &cmd).is_err());
    }

    #[test]
    fn test_decode_tip_present_ascii() {
        let config = default_config();
        // ASCII address '4' = 0x34
        let response = b"/40T1E\x42";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["tip_present"], true);
    }

    #[test]
    fn test_decode_tip_absent_ascii() {
        let config = default_config();
        let response = b"/40T0E\x42";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["tip_present"], false);
    }

    #[test]
    fn test_decode_tip_present_binary_addr() {
        let config = default_config(); // address = 4
        // Binary address 0x04 instead of ASCII '4' (0x34)
        let response = b"/\x04\x00T1\x00\x00\x00\x00\x00\x00\x00E\xFC";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["tip_present"], true);
    }

    #[test]
    fn test_decode_tip_absent_binary_addr() {
        let config = default_config();
        // Binary address 0x04, T0 = tip absent
        let response = b"/\x04T0\x00\x00\x00\x00\x00\x00\x00E\xFC";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["tip_present"], false);
    }

    #[test]
    fn test_decode_concatenated_frames() {
        let config = default_config();
        // Real hardware response: two frames concatenated.
        // Frame 1: /[0x04][0x06][0x0A]0[nulls]E[B8]
        // Frame 2: /[0x04]T0[nulls]E[FC]
        // We should pick frame 2 (last match) and decode T0.
        let response: &[u8] = &[
            0x2F, 0x04, 0x06, 0x0A, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x45, 0xB8,
            0x2F, 0x04, 0x54, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x45, 0xFC,
        ];
        let props = decode(&config, response).unwrap();
        assert_eq!(props["tip_present"], false); // T0 from frame 2
    }

    #[test]
    fn test_decode_wrong_address_returns_none() {
        let config = default_config(); // address = 4
        let response = b"/50T1E\x42"; // ASCII address 5
        assert!(decode(&config, response).is_none());
    }

    #[test]
    fn test_decode_wrong_binary_address_returns_none() {
        let config = default_config(); // address = 4
        let response = b"/\x05\x00T1E\x42"; // binary address 5
        assert!(decode(&config, response).is_none());
    }

    #[test]
    fn test_decode_status_no_error() {
        let config = default_config();
        let response = b"/40dataE\x42";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["status"], "no_error");
    }

    #[test]
    fn test_decode_binary_status_code() {
        let config = default_config();
        // Binary address 0x04, binary status 0x06 = not_initialized
        let response = b"/\x04\x06\x0A0\x00\x00\x00\x00\x00\x00E\xB8";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["status"], "not_initialized");
    }
}
