//! SC9300 main-controller serial protocol.
//!
//! Protocol source: ScienceOL/Liyan bc4a6e94648838d3c991b8c8472eaad63bc62121,
//! plugins/sc9300/internal/device/{controller,port,protocol}.go.
//! Reply behavior cross-checked against Mile-Away/sc9300
//! ca5f3692969e8b19960c698b620efcfba0745b59, sc9300-rs/src/{lib,serial,protocol}.rs.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{timeout, timeout_at, Instant};

use super::actions::Axis;
use super::calibration::{millimetres, XY_PITCH_MM, Z_PITCH_MM};

const PREAMBLE: [u8; 2] = [0x5a, 0x58];
const FOOTER: [u8; 2] = [0x0d, 0x0a];
const OVERHEAD: usize = 20;
const MAX_PAYLOAD: usize = 4096;
const DEVICE_TYPE: u16 = 0x00d1;
const ADDRESS: u8 = 1;

const READ_DEADLINE: Duration = Duration::from_secs(2);
const ACK_DEADLINE: Duration = Duration::from_millis(250);
const MAX_COMMAND_PAYLOAD: usize = 85;

#[derive(Clone, Copy)]
#[repr(u16)]
pub(super) enum Command {
    Status = 0x0000,
    PickUpTips = 0x0001,
    SetBackTips = 0x0002,
    Aspirate = 0x0003,
    Dispense = 0x0004,
    Homogenize = 0x0005,
    Clamp = 0x0006,
    Temperature = 0x0007,
    Oscillate = 0x0009,
    TemperatureAndOscillate = 0x000a,
    MagneticStand = 0x000b,
    Stop = 0x0100,
    Reset = 0x0101,
    Move = 0x0102,
    GetPosition = 0x0103,
    Enable = 0x0104,
    ReadInfo = 0x0105,
    Cabinet = 0x0107,
    CheckLimit = 0x0109,
    ClearError = 0x010a,
    Jog = 0x010c,
}

impl Command {
    fn response(self) -> u16 {
        self as u16 | 0x8000
    }

    fn expects_ack(self) -> bool {
        !matches!(self, Self::Enable | Self::ClearError)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WireIdentity {
    kind: u16,
    number: u32,
}

const BROADCAST: WireIdentity = WireIdentity {
    kind: 0xffff,
    number: 0xfffffff1,
};

#[derive(Clone, Copy)]
pub(super) struct Identity {
    wire: WireIdentity,
    firmware: [u8; 4],
}

#[derive(Serialize)]
pub(super) struct DeviceInfo {
    device_type: String,
    device_id: String,
    firmware: String,
}

impl DeviceInfo {
    pub(super) fn command_data(&self) -> Value {
        // device_id in the command envelope identifies the OpenSDL device.
        json!({
            "device_type": self.device_type,
            "controller_device_id": self.device_id,
            "firmware": self.firmware,
        })
    }
}

impl Identity {
    pub(super) fn observation(&self) -> Map<String, Value> {
        let [major, minor, patch, revision] = self.firmware;
        Map::from_iter([
            ("sc9300_firmware_major".into(), json!(major)),
            ("sc9300_firmware_minor".into(), json!(minor)),
            ("sc9300_firmware_patch".into(), json!(patch)),
            ("sc9300_firmware_revision".into(), json!(revision)),
        ])
    }

    pub(super) fn info(self) -> DeviceInfo {
        let [major, minor, patch, revision] = self.firmware;
        DeviceInfo {
            device_type: format!("{:04X}", self.wire.kind),
            device_id: format!("{:08X}", self.wire.number),
            firmware: format!("{major}.{minor}.{patch}.{revision}"),
        }
    }
}

#[derive(PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExecutionState {
    Success,
    InProgress,
    Failure,
    Unknown,
}

#[derive(Serialize)]
pub(super) struct Status {
    command_serial: u32,
    command: u16,
    state: ExecutionState,
    state_code: u8,
    error_code: u8,
}

impl Status {
    pub(super) fn observation(&self) -> Map<String, Value> {
        Map::from_iter([
            ("sc9300_command_serial".into(), json!(self.command_serial)),
            ("sc9300_command_code".into(), json!(self.command)),
            ("sc9300_state_code".into(), json!(self.state_code)),
            ("sc9300_fault_code".into(), json!(self.error_code)),
        ])
    }

    pub(super) fn idle(&self) -> bool {
        self.command == 0 && self.state == ExecutionState::Success && self.error_code == 0
    }

    pub(super) fn faulted(&self) -> bool {
        self.error_code != 0 || self.state == ExecutionState::Failure
    }
}

#[derive(Serialize)]
pub(super) struct Position {
    axis: Axis,
    x_mm: f64,
    y_mm: f64,
    z_mm: f64,
    x_pulses: i32,
    y_pulses: i32,
    z_pulses: i32,
    aux_a_pulses: i32,
    aux_b_pulses: i32,
}

impl Position {
    pub(super) fn observation(&self) -> impl Iterator<Item = (String, Value)> {
        let keys = match self.axis {
            Axis::Left => ["sc9300_left_x_mm", "sc9300_left_y_mm", "sc9300_left_z_mm"],
            Axis::Right => [
                "sc9300_right_x_mm",
                "sc9300_right_y_mm",
                "sc9300_right_z_mm",
            ],
            Axis::Gripper => [
                "sc9300_gripper_x_mm",
                "sc9300_gripper_y_mm",
                "sc9300_gripper_z_mm",
            ],
        };
        keys.into_iter()
            .zip([self.x_mm, self.y_mm, self.z_mm])
            .map(|(key, value)| (key.to_owned(), json!(value)))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum DispatchReply {
    MatchingResponse,
    NoResponse,
    NotExpected,
    ConnectionLost,
}

#[derive(Serialize)]
pub(super) struct Dispatch {
    command_serial: u32,
    reply: DispatchReply,
}

impl Dispatch {
    pub(super) fn connection_lost(&self) -> bool {
        matches!(self.reply, DispatchReply::ConnectionLost)
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("SC9300 connection unavailable")]
    Unavailable,
    #[error("SC9300 serial exchange timed out")]
    Timeout,
    #[error("serial endpoint did not identify the configured SC9300")]
    Identity,
}

pub(super) struct Client<T> {
    link: T,
    sequence: u32,
    pending: Vec<u8>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> Client<T> {
    pub(super) fn new(link: T) -> Self {
        Self {
            link,
            sequence: 10_000,
            pending: Vec::new(),
        }
    }

    pub(super) async fn identify(&mut self, expected: Option<Identity>) -> Result<Identity, Error> {
        let target = expected.map_or(BROADCAST, |identity| identity.wire);
        let limit = if expected.is_some() {
            READ_DEADLINE
        } else {
            Duration::from_millis(700)
        };
        let [kind1, kind2, id1, id2, id3, id4, major, minor, patch, revision] = self
            .exchange(target, Command::ReadInfo, &[], limit, |_| true)
            .await?;
        let wire = WireIdentity {
            kind: u16::from_be_bytes([kind1, kind2]),
            number: u32::from_be_bytes([id1, id2, id3, id4]),
        };
        if wire.kind != DEVICE_TYPE
            || wire.number == 0
            || expected.is_some_and(|identity| identity.wire != wire)
        {
            return Err(Error::Identity);
        }
        Ok(Identity {
            wire,
            firmware: [major, minor, patch, revision],
        })
    }

    pub(super) async fn status(&mut self, identity: Identity) -> Result<Status, Error> {
        let [serial1, serial2, serial3, serial4, command1, command2, state_code, error_code] = self
            .exchange(identity.wire, Command::Status, &[], READ_DEADLINE, |_| true)
            .await?;
        Ok(Status {
            command_serial: u32::from_be_bytes([serial1, serial2, serial3, serial4]),
            command: u16::from_be_bytes([command1, command2]),
            state: match state_code {
                0 => ExecutionState::Success,
                1 => ExecutionState::InProgress,
                0xff => ExecutionState::Failure,
                _ => ExecutionState::Unknown,
            },
            state_code,
            error_code,
        })
    }

    pub(super) async fn position(
        &mut self,
        identity: Identity,
        axis: Axis,
    ) -> Result<Position, Error> {
        let [_, coordinates @ ..]: [u8; 21] = self
            .exchange(
                identity.wire,
                Command::GetPosition,
                &[axis as u8],
                READ_DEADLINE,
                |payload| payload[0] == axis as u8,
            )
            .await?;
        let mut values = coordinates
            .chunks_exact(4)
            .map(|word| i32::from_be_bytes(word.try_into().expect("four-byte coordinate")));
        let x_pulses = values.next().unwrap();
        let y_pulses = values.next().unwrap();
        let z_pulses = values.next().unwrap();
        Ok(Position {
            axis,
            x_mm: millimetres(x_pulses, XY_PITCH_MM),
            y_mm: millimetres(y_pulses, XY_PITCH_MM),
            z_mm: millimetres(z_pulses, Z_PITCH_MM),
            x_pulses,
            y_pulses,
            z_pulses,
            aux_a_pulses: values.next().unwrap(),
            aux_b_pulses: values.next().unwrap(),
        })
    }

    pub(super) async fn dispatch(
        &mut self,
        identity: Identity,
        command: Command,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<Dispatch, Error> {
        let command_serial = timeout_at(
            deadline.min(Instant::now() + READ_DEADLINE),
            self.transmit(identity.wire, command, payload),
        )
        .await
        .map_err(|_| Error::Timeout)??;
        // The controller can start a command without acknowledging it. Motor
        // enable and clear error are known to be silent on the reference machine.
        // Stop can reply with the controller's full status payload.
        let reply = if command.expects_ack() {
            match timeout_at(
                deadline.min(Instant::now() + ACK_DEADLINE),
                self.receive::<1>(identity.wire, command.response(), |payload| {
                    payload.len() == 1 || (matches!(command, Command::Stop) && payload.len() >= 8)
                }),
            )
            .await
            {
                Ok(Ok(_)) => DispatchReply::MatchingResponse,
                Ok(Err(_)) => DispatchReply::ConnectionLost,
                Err(_) => DispatchReply::NoResponse,
            }
        } else {
            DispatchReply::NotExpected
        };
        Ok(Dispatch {
            command_serial,
            reply,
        })
    }

    async fn exchange<const N: usize>(
        &mut self,
        target: WireIdentity,
        command: Command,
        payload: &[u8],
        deadline: Duration,
        accepts: impl Fn(&[u8]) -> bool,
    ) -> Result<[u8; N], Error> {
        timeout(deadline, async {
            self.transmit(target, command, payload).await?;
            self.receive(target, command.response(), accepts).await
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn transmit(
        &mut self,
        target: WireIdentity,
        command: Command,
        payload: &[u8],
    ) -> Result<u32, Error> {
        self.pending.clear();
        let mut buffer = [0; OVERHEAD + MAX_COMMAND_PAYLOAD];
        let sequence = self.sequence;
        self.sequence = sequence.wrapping_add(1);
        let request = &mut buffer[..OVERHEAD + payload.len()];
        request[..2].copy_from_slice(&PREAMBLE);
        request[2..4].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        request[4..6].copy_from_slice(&target.kind.to_be_bytes());
        request[6..10].copy_from_slice(&target.number.to_be_bytes());
        request[10] = ADDRESS;
        request[11..15].copy_from_slice(&sequence.to_be_bytes());
        request[15..17].copy_from_slice(&(command as u16).to_be_bytes());
        let payload_end = 17 + payload.len();
        request[17..payload_end].copy_from_slice(payload);
        request[payload_end] = checksum(&request[..payload_end]);
        request[payload_end + 1..].copy_from_slice(&FOOTER);
        self.link
            .write_all(request)
            .await
            .map_err(|_| Error::Unavailable)?;
        Ok(sequence)
    }

    async fn receive<const N: usize>(
        &mut self,
        target: WireIdentity,
        response: u16,
        accepts: impl Fn(&[u8]) -> bool,
    ) -> Result<[u8; N], Error> {
        let mut scratch = [0; 512];
        loop {
            if let Some(reply) = take_reply(&mut self.pending, target, response, &accepts) {
                return Ok(reply);
            }
            let count = self
                .link
                .read(&mut scratch)
                .await
                .map_err(|_| Error::Unavailable)?;
            if count == 0 {
                return Err(Error::Unavailable);
            }
            self.pending.extend_from_slice(&scratch[..count]);
        }
    }
}

fn header_matches(frame: &[u8], target: WireIdentity) -> bool {
    frame[4..6] == target.kind.to_be_bytes() && frame[6..10] == target.number.to_be_bytes()
}

fn checksum(data: &[u8]) -> u8 {
    data.iter()
        .copied()
        .fold(0u8, u8::wrapping_add)
        .wrapping_neg()
}

fn take_reply<const N: usize>(
    pending: &mut Vec<u8>,
    target: WireIdentity,
    response: u16,
    accepts: impl Fn(&[u8]) -> bool,
) -> Option<[u8; N]> {
    let mut offset = 0;
    while let Some(relative) = pending[offset..].windows(2).position(|v| v == PREAMBLE) {
        offset += relative;
        let tail = &pending[offset..];
        if tail.len() < OVERHEAD {
            pending.drain(..offset);
            return None;
        }
        let length = usize::from(u16::from_be_bytes([tail[2], tail[3]]));
        if length <= MAX_PAYLOAD {
            let total = OVERHEAD + length;
            if tail.len() < total {
                pending.drain(..offset);
                return None;
            }
            let frame = &tail[..total];
            if frame[total - 2..] == FOOTER && frame[total - 3] == checksum(&frame[..total - 3]) {
                if length >= N
                    && frame[10] == ADDRESS
                    && frame[15..17] == response.to_be_bytes()
                    && (header_matches(frame, BROADCAST) || header_matches(frame, target))
                    && accepts(&frame[17..17 + length])
                {
                    let mut reply = [0; N];
                    reply.copy_from_slice(&frame[17..17 + N]);
                    pending.drain(..offset + total);
                    return Some(reply);
                }
                offset += total;
                continue;
            }
        }
        offset += 1;
    }
    let keep = usize::from(pending.last() == Some(&PREAMBLE[0]));
    pending.drain(..pending.len() - keep);
    None
}

#[cfg(test)]
pub(super) mod tests;
