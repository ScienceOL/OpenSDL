//! Node → mother registration announcement.
//!
//! A REG frame is the node telling the dongle "I exist; here is my identity."
//! Two formats coexist during the transition off compiled-in hardware_id:
//!
//! - **New (preferred):** `REG` alone (4 bytes). Mother looks up the node's MAC
//!   in `OsdlConfig.mac_assignments` to resolve the station.
//! - **Legacy:** `REG <hardware_id>`. Mother takes the hardware_id straight
//!   from the frame. Kept so old firmware (e.g. the LilyGO LCD chinwe board)
//!   still REGs cleanly while we roll out the new mechanism.
//!
//! The new format omits the trailing space on purpose so a parser can tell
//! "no hw_id" from "empty hw_id" cheaply.

use alloc::string::String;
use alloc::vec::Vec;

const PREFIX_WITH_SPACE: &[u8] = b"REG ";
const PREFIX_BARE: &[u8] = b"REG";

/// What a REG payload announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reg {
    /// New form. The mother resolves identity via `mac_assignments[mac]`.
    MacOnly,
    /// Legacy form: the firmware baked a `hardware_id` in.
    WithHardwareId(String),
}

/// Build the new (MAC-only) REG payload.
pub fn build_mac_only() -> Vec<u8> {
    PREFIX_BARE.to_vec()
}

/// Build a legacy REG payload carrying a hardware_id.
pub fn build_with_hardware_id(hardware_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(PREFIX_WITH_SPACE.len() + hardware_id.len());
    buf.extend_from_slice(PREFIX_WITH_SPACE);
    buf.extend_from_slice(hardware_id.as_bytes());
    buf
}

/// Try to parse an ESP-NOW *payload* (the bytes after the 6-byte dst_mac
/// header) as a REG announcement. Returns `None` if the payload isn't a REG
/// frame at all.
///
/// Rules:
/// - `REG` exactly → `Reg::MacOnly`
/// - `REG <hw_id>` (`REG ` + non-empty utf8) → `Reg::WithHardwareId(...)`
/// - anything else (including `REG ` with empty hw_id, or non-utf8 hw_id) → None
pub fn parse(payload: &[u8]) -> Option<Reg> {
    if payload == PREFIX_BARE {
        return Some(Reg::MacOnly);
    }
    if !payload.starts_with(PREFIX_WITH_SPACE) {
        return None;
    }
    let id_bytes = &payload[PREFIX_WITH_SPACE.len()..];
    if id_bytes.is_empty() {
        return None;
    }
    let hw_id = core::str::from_utf8(id_bytes).ok()?;
    Some(Reg::WithHardwareId(hw_id.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mac_only() {
        assert_eq!(parse(b"REG"), Some(Reg::MacOnly));
    }

    #[test]
    fn parse_legacy_with_hw_id() {
        assert_eq!(
            parse(b"REG bus.laiyu_xyz.station1"),
            Some(Reg::WithHardwareId("bus.laiyu_xyz.station1".into()))
        );
    }

    #[test]
    fn parse_rejects_unrelated_payload() {
        assert!(parse(b"hello").is_none());
        assert!(parse(b"").is_none());
        assert!(parse(b"REGISTER").is_none()); // wrong prefix
    }

    #[test]
    fn parse_rejects_empty_hw_id() {
        // "REG " with nothing after is ambiguous — reject so callers don't
        // silently register an empty hardware_id.
        assert!(parse(b"REG ").is_none());
    }

    #[test]
    fn parse_rejects_non_utf8_hw_id() {
        let mut bad = b"REG ".to_vec();
        bad.push(0xFF);
        bad.push(0xFE);
        assert!(parse(&bad).is_none());
    }

    #[test]
    fn build_then_parse_roundtrip_mac_only() {
        let bytes = build_mac_only();
        assert_eq!(parse(&bytes), Some(Reg::MacOnly));
    }

    #[test]
    fn build_then_parse_roundtrip_legacy() {
        let bytes = build_with_hardware_id("syringe_pump_with_valve.runze.SY03B-T06");
        assert_eq!(
            parse(&bytes),
            Some(Reg::WithHardwareId(
                "syringe_pump_with_valve.runze.SY03B-T06".into()
            ))
        );
    }
}
