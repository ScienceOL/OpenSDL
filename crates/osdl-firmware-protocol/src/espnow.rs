//! ESP-NOW frame layout used between dongle and nodes.
//!
//! Every directed frame is `[dst_mac(6) | payload(...)]`, broadcast on the
//! shared MAC. The receiver self-filters by comparing the leading 6 bytes
//! against its own MAC.

use alloc::vec::Vec;

use crate::Mac;

/// Build a directed ESP-NOW frame: 6-byte destination MAC followed by `payload`.
pub fn build_frame(dst: &Mac, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(6 + payload.len());
    buf.extend_from_slice(dst);
    buf.extend_from_slice(payload);
    buf
}

/// Split an inbound ESP-NOW frame into `(dst_mac, payload)`. Returns `None` if
/// the frame is shorter than the 6-byte header.
pub fn parse_frame(frame: &[u8]) -> Option<(&Mac, &[u8])> {
    if frame.len() < 6 {
        return None;
    }
    let dst: &Mac = frame[..6].try_into().ok()?;
    Some((dst, &frame[6..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_then_parse_roundtrip() {
        let dst: Mac = [0x30, 0xED, 0xA0, 0xB6, 0x5B, 0x38];
        let payload: &[u8] = b"hello";
        let frame = build_frame(&dst, payload);
        assert_eq!(frame.len(), 11);
        let (got_dst, got_payload) = parse_frame(&frame).unwrap();
        assert_eq!(got_dst, &dst);
        assert_eq!(got_payload, payload);
    }

    #[test]
    fn parse_too_short_returns_none() {
        assert!(parse_frame(&[]).is_none());
        assert!(parse_frame(&[0; 5]).is_none());
    }

    #[test]
    fn parse_exactly_six_bytes_yields_empty_payload() {
        let dst: Mac = [1, 2, 3, 4, 5, 6];
        let (got_dst, payload) = parse_frame(&dst).unwrap();
        assert_eq!(got_dst, &dst);
        assert!(payload.is_empty());
    }
}
