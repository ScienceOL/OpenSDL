//! Runze syringe pump serial protocol codec.
//!
//! Protocol: ASCII over RS-485, 9600 baud.
//!   Send:    /{address}{command}R\r\n   (R = execute, omit R for queries)
//!   Receive: ASCII string ending with \n, status byte at [0], data at [3:-3]
//!
//! Models: SY03B-T06 (6-port valve), SY03B-T08 (8-port valve)
//! Both share the same protocol, differing only in valve port count.

use crate::protocol::DeviceCommand;
use std::collections::HashMap;

/// Device-specific config stored alongside the registry entry.
#[derive(Debug, Clone)]
pub struct RunzeConfig {
    pub address: String,
    pub max_volume: f64,
    pub total_steps: u32,
    pub total_steps_vel: u32,
    /// Line ending for serial frames. Default "\r\n" for RS-485;
    /// ChinWe TCP pumps use "\r".
    pub line_ending: String,
}

impl Default for RunzeConfig {
    fn default() -> Self {
        Self {
            address: "1".into(),
            max_volume: 25.0,
            total_steps: 6000,
            total_steps_vel: 6000,
            line_ending: "\r\n".into(),
        }
    }
}

/// Encode a high-level command into serial bytes for the Runze pump.
pub fn encode(config: &RunzeConfig, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
    let addr = &config.address;
    let le = &config.line_ending;

    let command_str = match cmd.action.as_str() {
        "initialize" => format!("/{}ZR{}", addr, le),

        "set_position" => {
            let position = cmd
                .params
                .get("position")
                .and_then(|v| v.as_f64())
                .ok_or("set_position requires 'position' (float, mL)")?;

            let pos_step = (position / config.max_volume * config.total_steps as f64) as u32;

            if let Some(vel) = cmd.params.get("max_velocity").and_then(|v| v.as_f64()) {
                let pulse_freq =
                    ((vel / config.max_volume * config.total_steps_vel as f64) as u32).min(6000);
                format!("/{}V{}A{}R{}", addr, pulse_freq, pos_step, le)
            } else {
                format!("/{}A{}R{}", addr, pos_step, le)
            }
        }

        "pull_plunger" => {
            let volume = cmd
                .params
                .get("volume")
                .and_then(|v| v.as_f64())
                .ok_or("pull_plunger requires 'volume' (float, mL)")?;
            let steps = (volume / config.max_volume * config.total_steps as f64) as u32;
            format!("/{}P{}R{}", addr, steps, le)
        }

        "push_plunger" => {
            let volume = cmd
                .params
                .get("volume")
                .and_then(|v| v.as_f64())
                .ok_or("push_plunger requires 'volume' (float, mL)")?;
            let steps = (volume / config.max_volume * config.total_steps as f64) as u32;
            format!("/{}D{}R{}", addr, steps, le)
        }

        "set_valve_position" => {
            let pos = cmd
                .params
                .get("position")
                .ok_or("set_valve_position requires 'position'")?;
            let pos_str = if let Some(n) = pos.as_u64() {
                format!("I{}", n)
            } else if let Some(s) = pos.as_str() {
                if s.len() == 1 && s.as_bytes()[0] > b'9' {
                    s.to_uppercase()
                } else {
                    format!("I{}", s)
                }
            } else {
                return Err("position must be an integer or string".into());
            };
            format!("/{}{}R{}", addr, pos_str, le)
        }

        "set_max_velocity" => {
            let vel = cmd
                .params
                .get("velocity")
                .and_then(|v| v.as_f64())
                .ok_or("set_max_velocity requires 'velocity' (float, mL/s)")?;
            let pulse_freq =
                ((vel / config.max_volume * config.total_steps_vel as f64) as u32).min(6000);
            format!("/{}V{}R{}", addr, pulse_freq, le)
        }

        "set_velocity_grade" => {
            let grade = cmd
                .params
                .get("velocity")
                .ok_or("set_velocity_grade requires 'velocity'")?;
            let grade_str = if let Some(n) = grade.as_u64() {
                n.to_string()
            } else if let Some(s) = grade.as_str() {
                s.to_string()
            } else {
                return Err("velocity must be an integer or string".into());
            };
            format!("/{}S{}R{}", addr, grade_str, le)
        }

        "stop" | "stop_operation" => format!("/{}TR{}", addr, le),

        "query_status" => format!("/{}Q{}", addr, le),
        "query_position" => format!("/{}?0{}", addr, le),
        "query_velocity" => format!("/{}?2{}", addr, le),
        "query_valve_position" => format!("/{}?6{}", addr, le),
        "query_software_version" => format!("/{}?23{}", addr, le),

        "send_command" => {
            // Raw command passthrough
            let raw = cmd
                .params
                .get("full_command")
                .and_then(|v| v.as_str())
                .ok_or("send_command requires 'full_command' (string)")?;
            raw.to_string()
        }

        other => return Err(format!("unknown action: {}", other)),
    };

    Ok(command_str.into_bytes())
}

/// Decode a serial response from the Runze pump into status properties.
///
/// Returns None if the response is incomplete or unparseable.
///
/// Response formats seen in the wild:
///   With delimiter: `[noise...] / <address> <status> [data...] \x03 \r\n`
///   Without delimiter (legacy): `<status> [data...] \r\n`
///
/// Status byte: `` ` `` (0x60) = Idle, `@` (0x40) = Busy/error.
/// Leading noise bytes (e.g., 0xFF on RS-485/TCP) are skipped.
pub fn decode(config: &RunzeConfig, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
    // Skip leading non-ASCII noise bytes (e.g., 0xFF from bus),
    // then find '/' delimiter for the framed format.
    let slash_pos = bytes.iter().position(|&b| b == b'/');

    // Parse from '/' if found, otherwise from the first valid byte
    let start = slash_pos.unwrap_or(0);
    let text = std::str::from_utf8(&bytes[start..]).ok()?;
    let text = text.trim_end_matches(['\r', '\n']);

    let (status_byte, data) = if slash_pos.is_some() {
        // Framed format: /<address><status>[data...][\x03]
        if text.len() < 3 {
            return None;
        }
        let status = text.as_bytes()[2]; // skip '/' + address
        let data_end = text.find('\x03').unwrap_or(text.len());
        let data = if data_end > 3 { &text[3..data_end] } else { "" };
        (status, data)
    } else {
        // Legacy format (no '/'): <status>[data...]
        if text.is_empty() {
            return None;
        }
        let status = text.as_bytes()[0];
        let rest = &text[1..];
        let data = rest.split('\x03').next().unwrap_or("");
        (status, data)
    };

    let mut props = HashMap::new();

    let is_idle = status_byte == b'`' || (status_byte & (1 << 5)) != 0;
    props.insert(
        "status".into(),
        serde_json::Value::String(if is_idle { "Idle" } else { "Busy" }.into()),
    );

    if !data.is_empty() {
        if let Ok(n) = data.parse::<f64>() {
            if n > 0.0 && n <= config.total_steps as f64 {
                let volume = n / config.total_steps as f64 * config.max_volume;
                props.insert("position".into(), serde_json::json!(volume));
            }
            props.insert("raw_value".into(), serde_json::json!(n));
        } else if data.len() == 1 {
            props.insert(
                "valve_position".into(),
                serde_json::Value::String(data.to_uppercase()),
            );
        } else {
            props.insert(
                "raw_data".into(),
                serde_json::Value::String(data.to_string()),
            );
        }
    }

    Some(props)
}

// --------------- Driver trait impl ---------------

use crate::driver::Driver;
use crate::driver::registry::DriverRegistry;

/// Runze syringe pump driver instance (pre-configured).
pub struct RunzeDriver {
    config: RunzeConfig,
}

impl Driver for RunzeDriver {
    fn name(&self) -> &str {
        "runze"
    }

    fn encode(&self, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        encode(&self.config, cmd)
    }

    fn decode(&self, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
        decode(&self.config, bytes)
    }
}

/// Create a RunzeDriver from YAML device config.
pub fn create_from_yaml(yaml: &serde_yaml::Value) -> Result<Box<dyn Driver>, String> {
    let mut cfg = RunzeConfig::default();
    if let Some(addr) = yaml.get("address").and_then(|v| v.as_str()) {
        cfg.address = addr.to_string();
    } else if let Some(addr) = yaml.get("address").and_then(|v| v.as_u64()) {
        cfg.address = addr.to_string();
    }
    if let Some(vol) = yaml.get("max_volume").and_then(|v| v.as_f64()) {
        cfg.max_volume = vol;
    }
    if let Some(steps) = yaml.get("total_steps").and_then(|v| v.as_u64()) {
        cfg.total_steps = steps as u32;
    }
    if let Some(steps) = yaml.get("total_steps_vel").and_then(|v| v.as_u64()) {
        cfg.total_steps_vel = steps as u32;
    }
    if let Some(le) = yaml.get("line_ending").and_then(|v| v.as_str()) {
        cfg.line_ending = le.replace("\\r", "\r").replace("\\n", "\n");
    }
    Ok(Box::new(RunzeDriver { config: cfg }))
}

/// Register this driver with the registry.
pub fn register(registry: &mut DriverRegistry) {
    registry.register("runze", create_from_yaml);
}

// --------------- Tests ---------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DeviceCommand;

    fn default_config() -> RunzeConfig {
        RunzeConfig::default()
    }

    fn make_cmd(action: &str, params: serde_json::Value) -> DeviceCommand {
        DeviceCommand {
            command_id: "test".into(),
            device_id: "pump-01".into(),
            action: action.into(),
            params,
        }
    }

    #[test]
    fn test_encode_initialize() {
        let config = default_config();
        let cmd = make_cmd("initialize", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1ZR\r\n");
    }

    #[test]
    fn test_encode_set_position() {
        let config = default_config();
        // 12.5 mL = 3000 steps (12.5 / 25.0 * 6000)
        let cmd = make_cmd("set_position", serde_json::json!({"position": 12.5}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1A3000R\r\n");
    }

    #[test]
    fn test_encode_set_position_with_velocity() {
        let config = default_config();
        // position 12.5 mL = 3000 steps
        // velocity 5.0 mL/s = 1200 pulse freq (5.0 / 25.0 * 6000)
        let cmd = make_cmd(
            "set_position",
            serde_json::json!({"position": 12.5, "max_velocity": 5.0}),
        );
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1V1200A3000R\r\n");
    }

    #[test]
    fn test_encode_pull_plunger() {
        let config = default_config();
        // 2.5 mL = 600 steps
        let cmd = make_cmd("pull_plunger", serde_json::json!({"volume": 2.5}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1P600R\r\n");
    }

    #[test]
    fn test_encode_push_plunger() {
        let config = default_config();
        let cmd = make_cmd("push_plunger", serde_json::json!({"volume": 2.5}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1D600R\r\n");
    }

    #[test]
    fn test_encode_set_valve_position_int() {
        let config = default_config();
        let cmd = make_cmd("set_valve_position", serde_json::json!({"position": 3}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1I3R\r\n");
    }

    #[test]
    fn test_encode_stop() {
        let config = default_config();
        let cmd = make_cmd("stop", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1TR\r\n");
    }

    #[test]
    fn test_encode_query_status() {
        let config = default_config();
        let cmd = make_cmd("query_status", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, b"/1Q\r\n");
    }

    #[test]
    fn test_decode_idle_status() {
        let config = default_config();
        // Simulated response: status=idle(`)
        let response = b"`\x00\x00`\x00\x00\n";
        let props = decode(&config, response);
        assert!(props.is_some());
        let props = props.unwrap();
        assert_eq!(props["status"], "Idle");
    }

    #[test]
    fn test_decode_position() {
        let config = default_config();
        // Simulated response: /0`3000\r\n  (address=0, status=`, data=3000)
        // After trim: "/0`3000" (len 7), data = text[3..7-3] would be wrong.
        // Use longer padding: "/0`0003000/0\n" so data slice = "003000" which won't parse right.
        // Simplest: short response falls into else branch (len <= 6), data = text[1..]
        let response = b"`3000\n";
        let props = decode(&config, response).unwrap();
        assert_eq!(props["status"], "Idle");
        // data = "3000", 3000 steps = 12.5 mL
        assert_eq!(props["position"], 12.5);
    }

    #[test]
    fn test_encode_unknown_action() {
        let config = default_config();
        let cmd = make_cmd("fly_to_moon", serde_json::json!({}));
        let result = encode(&config, &cmd);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown action"));
    }
}
