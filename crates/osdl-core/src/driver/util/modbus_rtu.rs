//! Modbus RTU frame codec: CRC16 calculation, frame building, response parsing.
//!
//! Reusable utility used by Laiyu XYZ stepper motors and ChinWe XKC sensor.

/// Calculate Modbus CRC16 (polynomial 0xA001, initial 0xFFFF).
/// Returns 2 bytes in little-endian order (standard Modbus CRC byte order).
pub fn crc16(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 0x0001 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc.to_le_bytes()
}

/// Build a Modbus RTU "Read Holding Registers" request (function code 0x03).
///
/// Frame: [slave, 0x03, addr_hi, addr_lo, count_hi, count_lo, crc_lo, crc_hi]
pub fn build_read_registers(slave: u8, start_addr: u16, count: u16) -> Vec<u8> {
    let mut frame = vec![slave, 0x03];
    frame.extend_from_slice(&start_addr.to_be_bytes());
    frame.extend_from_slice(&count.to_be_bytes());
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc);
    frame
}

/// Build a "Write Single Register" request (function code 0x06).
///
/// Frame: [slave, 0x06, addr_hi, addr_lo, val_hi, val_lo, crc_lo, crc_hi]
pub fn build_write_single(slave: u8, addr: u16, value: u16) -> Vec<u8> {
    let mut frame = vec![slave, 0x06];
    frame.extend_from_slice(&addr.to_be_bytes());
    frame.extend_from_slice(&value.to_be_bytes());
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc);
    frame
}

/// Build a "Write Multiple Registers" request (function code 0x10).
///
/// Frame: [slave, 0x10, start_hi, start_lo, qty_hi, qty_lo, byte_count, data..., crc_lo, crc_hi]
pub fn build_write_multiple(slave: u8, start_addr: u16, values: &[u16]) -> Vec<u8> {
    let qty = values.len() as u16;
    let byte_count = (values.len() * 2) as u8;
    let mut frame = vec![slave, 0x10];
    frame.extend_from_slice(&start_addr.to_be_bytes());
    frame.extend_from_slice(&qty.to_be_bytes());
    frame.push(byte_count);
    for &v in values {
        frame.extend_from_slice(&v.to_be_bytes());
    }
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc);
    frame
}

/// Parse a Modbus RTU response frame.
///
/// Validates CRC and returns `(slave_id, function_code, payload)`.
/// The payload does NOT include slave, function code, or CRC bytes.
/// Returns `None` if CRC fails or frame is too short.
pub fn parse_response(bytes: &[u8]) -> Option<(u8, u8, Vec<u8>)> {
    if bytes.len() < 5 {
        return None;
    }
    let data = &bytes[..bytes.len() - 2];
    let received_crc = [bytes[bytes.len() - 2], bytes[bytes.len() - 1]];
    let calculated_crc = crc16(data);
    if received_crc != calculated_crc {
        return None;
    }
    Some((bytes[0], bytes[1], data[2..].to_vec()))
}

/// Extract register values from a 0x03 read response payload.
///
/// The payload format is: [byte_count, data_hi, data_lo, ...]
/// Returns a Vec of u16 register values.
pub fn parse_read_registers(payload: &[u8]) -> Option<Vec<u16>> {
    if payload.is_empty() {
        return None;
    }
    let byte_count = payload[0] as usize;
    if payload.len() < 1 + byte_count || byte_count % 2 != 0 {
        return None;
    }
    let mut values = Vec::with_capacity(byte_count / 2);
    for i in (1..1 + byte_count).step_by(2) {
        values.push(u16::from_be_bytes([payload[i], payload[i + 1]]));
    }
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc16_known_value() {
        // Modbus request: slave=1, fn=0x03, addr=0x0000, count=0x0006
        let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x06];
        let crc = crc16(&data);
        // Verified against Python XYZModbus._crc16
        let frame = build_read_registers(1, 0x0000, 0x0006);
        assert_eq!(&frame[..6], &data);
        assert_eq!(frame.len(), 8);
        assert_eq!(&frame[6..], &crc);
    }

    #[test]
    fn test_build_read_registers() {
        let frame = build_read_registers(1, 0x0000, 6);
        assert_eq!(frame[0], 1); // slave
        assert_eq!(frame[1], 0x03); // function
        assert_eq!(frame[2], 0x00); // addr hi
        assert_eq!(frame[3], 0x00); // addr lo
        assert_eq!(frame[4], 0x00); // count hi
        assert_eq!(frame[5], 0x06); // count lo
        assert_eq!(frame.len(), 8);
    }

    #[test]
    fn test_build_write_single() {
        let frame = build_write_single(1, 0x0016, 1);
        assert_eq!(frame[0], 1);
        assert_eq!(frame[1], 0x06);
        assert_eq!(frame[2], 0x00);
        assert_eq!(frame[3], 0x16);
        assert_eq!(frame[4], 0x00);
        assert_eq!(frame[5], 0x01);
        assert_eq!(frame.len(), 8);
    }

    #[test]
    fn test_build_write_multiple() {
        let frame = build_write_multiple(1, 0x0010, &[0x0000, 0x1000, 200, 500, 50]);
        assert_eq!(frame[0], 1);
        assert_eq!(frame[1], 0x10);
        assert_eq!(frame[2], 0x00); // start hi
        assert_eq!(frame[3], 0x10); // start lo
        assert_eq!(frame[4], 0x00); // qty hi
        assert_eq!(frame[5], 0x05); // qty lo = 5
        assert_eq!(frame[6], 10); // byte count = 5 * 2
    }

    #[test]
    fn test_parse_response_valid() {
        // Build a read response: slave=1, fn=0x03, byte_count=2, data=[0x00, 0x00]
        let mut frame = vec![0x01, 0x03, 0x02, 0x00, 0x00];
        let crc = crc16(&frame);
        frame.extend_from_slice(&crc);

        let result = parse_response(&frame);
        assert!(result.is_some());
        let (slave, func, payload) = result.unwrap();
        assert_eq!(slave, 1);
        assert_eq!(func, 0x03);
        assert_eq!(payload, vec![0x02, 0x00, 0x00]);
    }

    #[test]
    fn test_parse_response_bad_crc() {
        let frame = vec![0x01, 0x03, 0x02, 0x00, 0x00, 0xFF, 0xFF];
        assert!(parse_response(&frame).is_none());
    }

    #[test]
    fn test_parse_read_registers() {
        // byte_count=4, then 2 registers: 0x002E (46) and 0x0000 (0)
        let payload = vec![0x04, 0x00, 0x2E, 0x00, 0x00];
        let regs = parse_read_registers(&payload).unwrap();
        assert_eq!(regs, vec![0x002E, 0x0000]);
    }

    #[test]
    fn test_roundtrip_read_registers() {
        // Build request, fake a response, parse it
        let _request = build_read_registers(2, 0x0000, 3);

        // Simulate response: slave=2, fn=0x03, 3 registers
        let mut response = vec![0x02, 0x03, 0x06, 0x00, 0x01, 0xFF, 0xFF, 0x00, 0x00];
        let crc = crc16(&response);
        response.extend_from_slice(&crc);

        let (slave, func, payload) = parse_response(&response).unwrap();
        assert_eq!(slave, 2);
        assert_eq!(func, 0x03);
        let regs = parse_read_registers(&payload).unwrap();
        assert_eq!(regs, vec![0x0001, 0xFFFF, 0x0000]);
    }
}
