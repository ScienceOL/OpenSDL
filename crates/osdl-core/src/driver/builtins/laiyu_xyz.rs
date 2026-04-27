//! Laiyu XYZ stepper motor codec (Modbus RTU, RS-485, 115200 baud).
//!
//! Three motors on slave addresses 1 (X), 2 (Y), 3 (Z).
//! Uses standard Modbus RTU: read_registers (0x03), write_single (0x06),
//! write_multiple (0x10).
//!
//! Coordinate system:
//!   STEPS_PER_REV = 16384
//!   X/Y: lead 80mm → 204.8 steps/mm
//!   Z:   lead 5mm  → 3276.8 steps/mm

use crate::driver::util::modbus_rtu;
use crate::protocol::DeviceCommand;
use std::collections::HashMap;

/// Register addresses (from hardware datasheet).
const REG_STATUS: u16 = 0x0000;
const REG_TARGET_HIGH: u16 = 0x0010;
const REG_START: u16 = 0x0016;
const REG_ENABLE: u16 = 0x0006;
const REG_ZERO_CMD: u16 = 0x000F;

/// Motor status codes.
const STATUS_STANDBY: u16 = 0x0000;
const STATUS_RUNNING: u16 = 0x0001;
const STATUS_COLLISION: u16 = 0x0002;
const STATUS_FORWARD_LIMIT: u16 = 0x0003;
const STATUS_REVERSE_LIMIT: u16 = 0x0004;

/// Device-specific configuration stored alongside the registry entry.
#[derive(Debug, Clone)]
pub struct LaiyuXyzConfig {
    pub slave_id: u8,
    pub axis: String,
    pub steps_per_rev: u32,
    pub lead_mm: f64,
}

impl LaiyuXyzConfig {
    pub fn steps_per_mm(&self) -> f64 {
        self.steps_per_rev as f64 / self.lead_mm
    }
}

impl Default for LaiyuXyzConfig {
    fn default() -> Self {
        Self {
            slave_id: 1,
            axis: "X".into(),
            steps_per_rev: 16384,
            lead_mm: 80.0,
        }
    }
}

/// Encode a high-level command into Modbus RTU bytes.
pub fn encode(config: &LaiyuXyzConfig, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
    match cmd.action.as_str() {
        "get_status" => {
            // Read 6 registers starting at REG_STATUS:
            // status, pos_high, pos_low, actual_speed, (emergency_stop), current
            Ok(modbus_rtu::build_read_registers(
                config.slave_id,
                REG_STATUS,
                6,
            ))
        }

        "move_to_position" => {
            // Write 5 registers: target_high, target_low, speed, accel, precision
            let position = cmd
                .params
                .get("position")
                .and_then(|v| v.as_i64())
                .ok_or("move_to_position requires 'position' (int, steps)")?
                as i32;
            let speed = cmd
                .params
                .get("speed")
                .and_then(|v| v.as_u64())
                .unwrap_or(2000) as u16;
            let accel = cmd
                .params
                .get("acceleration")
                .and_then(|v| v.as_u64())
                .unwrap_or(500) as u16;
            let precision = cmd
                .params
                .get("precision")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as u16;

            let pos_u32 = position as u32;
            let pos_high = (pos_u32 >> 16) as u16;
            let pos_low = (pos_u32 & 0xFFFF) as u16;

            let values = vec![pos_high, pos_low, speed, accel, precision];
            Ok(modbus_rtu::build_write_multiple(
                config.slave_id,
                REG_TARGET_HIGH,
                &values,
            ))
        }

        "start_motion" => Ok(modbus_rtu::build_write_single(
            config.slave_id,
            REG_START,
            1,
        )),

        "enable" => {
            let on = cmd
                .params
                .get("enable")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(modbus_rtu::build_write_single(
                config.slave_id,
                REG_ENABLE,
                if on { 1 } else { 0 },
            ))
        }

        "home" => Ok(modbus_rtu::build_write_single(
            config.slave_id,
            REG_ZERO_CMD,
            1,
        )),

        other => Err(format!("unknown laiyu_xyz action: {}", other)),
    }
}

/// Decode a Modbus RTU response into status properties.
///
/// Returns `None` if the response is not addressed to this slave or
/// the frame is invalid.
pub fn decode(config: &LaiyuXyzConfig, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
    let (slave, func, payload) = modbus_rtu::parse_response(bytes)?;

    // Only decode responses addressed to our slave
    if slave != config.slave_id {
        return None;
    }

    let mut props = HashMap::new();
    props.insert("axis".into(), serde_json::json!(config.axis));

    match func {
        0x03 => {
            // Read registers response
            let registers = modbus_rtu::parse_read_registers(&payload)?;
            if registers.len() >= 6 {
                let status = registers[0];
                let pos_high = registers[1];
                let pos_low = registers[2];
                let speed = registers[3] as i16; // signed
                let current = registers[5];

                // Reconstruct signed 32-bit position
                let position = ((pos_high as u32) << 16 | pos_low as u32) as i32;
                let mm = position as f64 / config.steps_per_mm();

                let status_str = match status {
                    STATUS_STANDBY => "standby",
                    STATUS_RUNNING => "running",
                    STATUS_COLLISION => "collision_stop",
                    STATUS_FORWARD_LIMIT => "forward_limit",
                    STATUS_REVERSE_LIMIT => "reverse_limit",
                    _ => "unknown",
                };

                props.insert("status".into(), serde_json::json!(status_str));
                props.insert("position_steps".into(), serde_json::json!(position));
                props.insert("position_mm".into(), serde_json::json!(mm));
                props.insert("speed".into(), serde_json::json!(speed));
                props.insert("current_ma".into(), serde_json::json!(current));
            }
        }
        0x06 | 0x10 => {
            // Write acknowledgment
            props.insert("status".into(), serde_json::json!("ack"));
        }
        _ => return None,
    }

    Some(props)
}

/// Convert work coordinate (mm) to machine steps, given a work origin offset.
pub fn work_mm_to_machine_steps(config: &LaiyuXyzConfig, mm: f64, origin_steps: i32) -> i32 {
    origin_steps + (mm * config.steps_per_mm()) as i32
}

/// Convert machine steps to work coordinate (mm), given a work origin offset.
pub fn machine_steps_to_work_mm(config: &LaiyuXyzConfig, steps: i32, origin_steps: i32) -> f64 {
    (steps - origin_steps) as f64 / config.steps_per_mm()
}

// --------------- Driver trait impl ---------------

use crate::driver::Driver;
use crate::driver::registry::DriverRegistry;

/// Laiyu XYZ stepper motor driver instance (pre-configured).
pub struct LaiyuXyzDriver {
    config: LaiyuXyzConfig,
}

impl Driver for LaiyuXyzDriver {
    fn name(&self) -> &str {
        "laiyu_xyz"
    }

    fn encode(&self, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        encode(&self.config, cmd)
    }

    fn decode(&self, bytes: &[u8]) -> Option<HashMap<String, serde_json::Value>> {
        decode(&self.config, bytes)
    }
}

/// Create a LaiyuXyzDriver from YAML device config.
pub fn create_from_yaml(yaml: &serde_yaml::Value) -> Result<Box<dyn Driver>, String> {
    let mut cfg = LaiyuXyzConfig::default();
    if let Some(id) = yaml.get("slave_id").and_then(|v| v.as_u64()) {
        cfg.slave_id = id as u8;
    }
    if let Some(axis) = yaml.get("axis").and_then(|v| v.as_str()) {
        cfg.axis = axis.to_string();
    }
    if let Some(spr) = yaml.get("steps_per_rev").and_then(|v| v.as_u64()) {
        cfg.steps_per_rev = spr as u32;
    }
    if let Some(lead) = yaml.get("lead_mm").and_then(|v| v.as_f64()) {
        cfg.lead_mm = lead;
    }
    Ok(Box::new(LaiyuXyzDriver { config: cfg }))
}

/// Register this driver with the registry.
pub fn register(registry: &mut DriverRegistry) {
    registry.register("laiyu_xyz", create_from_yaml);
}

// --------------- Tests ---------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DeviceCommand;

    fn make_cmd(action: &str, params: serde_json::Value) -> DeviceCommand {
        DeviceCommand {
            command_id: "test".into(),
            device_id: "xyz-x".into(),
            action: action.into(),
            params,
        }
    }

    fn x_config() -> LaiyuXyzConfig {
        LaiyuXyzConfig {
            slave_id: 1,
            axis: "X".into(),
            steps_per_rev: 16384,
            lead_mm: 80.0,
        }
    }

    fn z_config() -> LaiyuXyzConfig {
        LaiyuXyzConfig {
            slave_id: 3,
            axis: "Z".into(),
            steps_per_rev: 16384,
            lead_mm: 5.0,
        }
    }

    #[test]
    fn test_steps_per_mm() {
        let x = x_config();
        assert!((x.steps_per_mm() - 204.8).abs() < 0.01);
        let z = z_config();
        assert!((z.steps_per_mm() - 3276.8).abs() < 0.01);
    }

    #[test]
    fn test_encode_get_status() {
        let config = x_config();
        let cmd = make_cmd("get_status", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes[0], 1); // slave 1
        assert_eq!(bytes[1], 0x03); // read regs
        assert_eq!(bytes[4], 0x00);
        assert_eq!(bytes[5], 0x06); // 6 registers
        assert_eq!(bytes.len(), 8);
    }

    #[test]
    fn test_encode_move_to_position() {
        let config = x_config();
        let cmd = make_cmd(
            "move_to_position",
            serde_json::json!({"position": 4096, "speed": 200, "acceleration": 500}),
        );
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes[0], 1); // slave 1
        assert_eq!(bytes[1], 0x10); // write multiple
        assert_eq!(bytes[2], 0x00);
        assert_eq!(bytes[3], 0x10); // REG_TARGET_HIGH
    }

    #[test]
    fn test_encode_start_motion() {
        let config = x_config();
        let cmd = make_cmd("start_motion", serde_json::json!({}));
        let bytes = encode(&config, &cmd).unwrap();
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[1], 0x06); // write single
        assert_eq!(bytes[2], 0x00);
        assert_eq!(bytes[3], 0x16); // REG_START
    }

    #[test]
    fn test_encode_unknown_action() {
        let config = x_config();
        let cmd = make_cmd("fly", serde_json::json!({}));
        assert!(encode(&config, &cmd).is_err());
    }

    #[test]
    fn test_decode_status_response() {
        let config = x_config();
        // Build fake response: slave=1, fn=0x03, 6 registers
        // status=0 (standby), pos_high=0, pos_low=2048, speed=100, emergency=0, current=50
        let mut frame = vec![
            0x01, 0x03, 12, // slave, func, byte_count=12
            0x00, 0x00, // status = standby
            0x00, 0x00, // pos_high
            0x08, 0x00, // pos_low = 2048
            0x00, 0x64, // speed = 100
            0x00, 0x00, // emergency_stop
            0x00, 0x32, // current = 50
        ];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);

        let props = decode(&config, &frame).unwrap();
        assert_eq!(props["axis"], "X");
        assert_eq!(props["status"], "standby");
        assert_eq!(props["position_steps"], 2048);
        assert_eq!(props["speed"], 100);
        assert_eq!(props["current_ma"], 50);
    }

    #[test]
    fn test_decode_wrong_slave_returns_none() {
        let config = x_config(); // slave_id = 1
        // Response from slave 2
        let mut frame = vec![0x02, 0x03, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);
        assert!(decode(&config, &frame).is_none());
    }

    #[test]
    fn test_decode_write_ack() {
        let config = x_config();
        // Write single response echo: slave=1, fn=0x06, addr, value
        let mut frame = vec![0x01, 0x06, 0x00, 0x16, 0x00, 0x01];
        let crc = modbus_rtu::crc16(&frame);
        frame.extend_from_slice(&crc);
        let props = decode(&config, &frame).unwrap();
        assert_eq!(props["status"], "ack");
    }

    #[test]
    fn test_coordinate_conversion() {
        let config = x_config();
        let origin = 11799;
        // 10mm from work origin
        let machine = work_mm_to_machine_steps(&config, 10.0, origin);
        assert_eq!(machine, 11799 + 2048); // 10 * 204.8 = 2048

        let mm = machine_steps_to_work_mm(&config, machine, origin);
        assert!((mm - 10.0).abs() < 0.01);
    }
}
