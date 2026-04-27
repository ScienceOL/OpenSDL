//! Emm V5.0 closed-loop stepper motor codec (binary protocol).
//!
//! Frame format: [device_id, func_code, data..., 0x6B]
//! The trailing 0x6B is a fixed terminator/checksum byte.
//!
//! Used by ChinWe workstation motors (ID 4, 5) over TCP.
//! Communication parameters: 9600 baud (serial) or TCP socket.

use crate::protocol::DeviceCommand;
use std::collections::HashMap;

/// Device-specific configuration.
#[derive(Debug, Clone)]
pub struct EmmConfig {
    /// Device address on the bus (typically 4 or 5).
    pub device_id: u8,
}

impl Default for EmmConfig {
    fn default() -> Self {
        Self { device_id: 4 }
    }
}

/// Build a complete Emm binary frame: [device_id, func_code, payload..., 0x6B]
fn build_frame(id: u8, func: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(3 + payload.len());
    frame.push(id);
    frame.push(func);
    frame.extend_from_slice(payload);
    frame.push(0x6B); // fixed terminator
    frame
}

/// Encode a high-level command into Emm binary protocol bytes.
pub fn encode(config: &EmmConfig, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
    match cmd.action.as_str() {
        "enable" => {
            let on = cmd
                .params
                .get("enable")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let state: u8 = if on { 1 } else { 0 };
            Ok(build_frame(config.device_id, 0xF3, &[0xAB, state, 0]))
        }

        "run_speed" => {
            let speed = cmd
                .params
                .get("speed")
                .and_then(|v| v.as_u64())
                .ok_or("run_speed requires 'speed' (RPM)")? as u16;
            let direction = cmd
                .params
                .get("direction")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u8;
            let accel = cmd
                .params
                .get("acceleration")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as u8;
            let sp = speed.to_be_bytes();
            Ok(build_frame(
                config.device_id,
                0xF6,
                &[direction, sp[0], sp[1], accel, 0],
            ))
        }

        "run_position" => {
            let pulses = cmd
                .params
                .get("pulses")
                .and_then(|v| v.as_u64())
                .ok_or("run_position requires 'pulses' (int)")? as u32;
            let speed = cmd
                .params
                .get("speed")
                .and_then(|v| v.as_u64())
                .ok_or("run_position requires 'speed' (RPM)")? as u16;
            let direction = cmd
                .params
                .get("direction")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u8;
            let accel = cmd
                .params
                .get("acceleration")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as u8;
            let absolute = cmd
                .params
                .get("absolute")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let sp = speed.to_be_bytes();
            let pl = pulses.to_be_bytes();
            let is_abs: u8 = if absolute { 1 } else { 0 };
            Ok(build_frame(
                config.device_id,
                0xFD,
                &[direction, sp[0], sp[1], accel, pl[0], pl[1], pl[2], pl[3], is_abs, 0],
            ))
        }

        "stop" => Ok(build_frame(config.device_id, 0xFE, &[0x98, 0])),

        "get_position" => Ok(build_frame(config.device_id, 0x32, &[])),

        "set_zero" => Ok(build_frame(config.device_id, 0x0A, &[])),

        other => Err(format!("unknown emm action: {}", other)),
    }
}

/// Decode an Emm binary response into status properties.
///
/// Returns `None` if the response is not addressed to this device
/// or the frame is too short.
pub fn decode(config: &EmmConfig, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
    if bytes.is_empty() || bytes[0] != config.device_id {
        return None;
    }

    let mut props = HashMap::new();

    if bytes.len() >= 8 && bytes[1] == 0x32 {
        // get_position response: [id, 0x32, sign, pos_bytes(4), 0x6B]
        let sign = bytes[2];
        if bytes.len() >= 7 {
            let val = u32::from_be_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]);
            let position = if sign == 1 {
                -(val as i64)
            } else {
                val as i64
            };
            props.insert("position".into(), serde_json::json!(position));
            props.insert("status".into(), serde_json::json!("ok"));
        }
    } else if bytes.len() >= 4 {
        // Generic acknowledgment
        props.insert("status".into(), serde_json::json!("ack"));
    } else {
        return None;
    }

    Some(props)
}

// --------------- Driver trait impl ---------------

use crate::driver::Driver;
use crate::driver::registry::DriverRegistry;

/// Emm V5.0 stepper motor driver instance (pre-configured).
pub struct EmmDriver {
    config: EmmConfig,
}

impl Driver for EmmDriver {
    fn name(&self) -> &str {
        "emm"
    }

    fn encode(&self, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        encode(&self.config, cmd)
    }

    fn decode(&self, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
        decode(&self.config, bytes)
    }
}

/// Create an EmmDriver from YAML device config.
pub fn create_from_yaml(yaml: &serde_yaml::Value) -> Result<Box<dyn Driver>, String> {
    let mut cfg = EmmConfig::default();
    if let Some(id) = yaml.get("device_id").and_then(|v| v.as_u64()) {
        cfg.device_id = id as u8;
    }
    Ok(Box::new(EmmDriver { config: cfg }))
}

/// Register this driver with the registry.
pub fn register(registry: &mut DriverRegistry) {
    registry.register("emm", create_from_yaml);
}

// --------------- Tests ---------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DeviceCommand;

    fn default_config() -> EmmConfig {
        EmmConfig { device_id: 4 }
    }

    fn make_cmd(action: &str, params: serde_json::Value) -> DeviceCommand {
        DeviceCommand {
            command_id: "test".into(),
            device_id: "motor-04".into(),
            action: action.into(),
            params,
        }
    }

    #[test]
    fn test_encode_enable_on() {
        let config = default_config();
        let cmd = make_cmd("enable", serde_json::json!({"enable": true}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, vec![4, 0xF3, 0xAB, 1, 0, 0x6B]);
    }

    #[test]
    fn test_encode_enable_off() {
        let config = default_config();
        let cmd = make_cmd("enable", serde_json::json!({"enable": false}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, vec![4, 0xF3, 0xAB, 0, 0, 0x6B]);
    }

    #[test]
    fn test_encode_stop() {
        let config = default_config();
        let cmd = make_cmd("stop", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, vec![4, 0xFE, 0x98, 0, 0x6B]);
    }

    #[test]
    fn test_encode_get_position() {
        let config = default_config();
        let cmd = make_cmd("get_position", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, vec![4, 0x32, 0x6B]);
    }

    #[test]
    fn test_encode_set_zero() {
        let config = default_config();
        let cmd = make_cmd("set_zero", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes, vec![4, 0x0A, 0x6B]);
    }

    #[test]
    fn test_encode_run_speed() {
        let config = default_config();
        let cmd = make_cmd(
            "run_speed",
            serde_json::json!({"speed": 60, "direction": 0, "acceleration": 10}),
        );
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes[0], 4); // device_id
        assert_eq!(bytes[1], 0xF6); // func
        assert_eq!(bytes[2], 0); // direction
        // speed=60 big-endian: 0x00, 0x3C
        assert_eq!(bytes[3], 0x00);
        assert_eq!(bytes[4], 0x3C);
        assert_eq!(bytes[5], 10); // accel
        assert_eq!(*bytes.last().unwrap(), 0x6B);
    }

    #[test]
    fn test_encode_run_position() {
        let config = default_config();
        let cmd = make_cmd(
            "run_position",
            serde_json::json!({"pulses": 800, "speed": 60, "direction": 0}),
        );
        let bytes = encode(&config, &cmd).unwrap();
        // Frame: [4, 0xFD, dir, speed_hi, speed_lo, accel, pulse_0..3, is_abs, 0, 0x6B]
        assert_eq!(bytes[0], 4); // device_id
        assert_eq!(bytes[1], 0xFD); // func
        assert_eq!(bytes[2], 0); // direction
        assert_eq!(bytes[3], 0x00); // speed hi
        assert_eq!(bytes[4], 0x3C); // speed lo = 60
        assert_eq!(bytes[5], 10); // accel (default)
        // pulses=800 big-endian: 0x00, 0x00, 0x03, 0x20
        assert_eq!(bytes[6], 0x00);
        assert_eq!(bytes[7], 0x00);
        assert_eq!(bytes[8], 0x03);
        assert_eq!(bytes[9], 0x20);
        assert_eq!(*bytes.last().unwrap(), 0x6B);
    }

    #[test]
    fn test_encode_unknown_action() {
        let config = default_config();
        let cmd = make_cmd("fly", serde_json::json!({}));
        assert!(encode(&config, &cmd).is_err());
    }

    #[test]
    fn test_decode_get_position_positive() {
        let config = default_config();
        // [id=4, func=0x32, sign=0(positive), pos=1000(4 bytes BE), 0x6B]
        let response = vec![4, 0x32, 0, 0x00, 0x00, 0x03, 0xE8, 0x6B];
        let props = decode(&config, &response).unwrap();
        assert_eq!(props["position"], 1000);
        assert_eq!(props["status"], "ok");
    }

    #[test]
    fn test_decode_get_position_negative() {
        let config = default_config();
        let response = vec![4, 0x32, 1, 0x00, 0x00, 0x03, 0xE8, 0x6B];
        let props = decode(&config, &response).unwrap();
        assert_eq!(props["position"], -1000);
    }

    #[test]
    fn test_decode_wrong_device_returns_none() {
        let config = default_config(); // device_id = 4
        let response = vec![5, 0x32, 0, 0x00, 0x00, 0x00, 0x00, 0x6B];
        assert!(decode(&config, &response).is_none());
    }

    #[test]
    fn test_decode_generic_ack() {
        let config = default_config();
        let response = vec![4, 0xF3, 0x01, 0x6B];
        let props = decode(&config, &response).unwrap();
        assert_eq!(props["status"], "ack");
    }
}
