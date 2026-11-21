//! SC9300 read-only main-controller protocol.
//!
//! Protocol source: ScienceOL/Liyan bc4a6e94648838d3c991b8c8472eaad63bc62121,
//! plugins/sc9300/internal/device/{controller,port,protocol}.go.

use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

const PREAMBLE: [u8; 2] = [0x5a, 0x58];
const FOOTER: [u8; 2] = [0x0d, 0x0a];
const OVERHEAD: usize = 20;
const MAX_PAYLOAD: usize = 4096;
const READ_INFO: u16 = 0x0105;
const STATUS: u16 = 0x0000;
const DEVICE_TYPE: u16 = 0x00d1;
const ADDRESS: u8 = 1;

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

impl Identity {
    pub(super) fn info(self) -> DeviceInfo {
        let [major, minor, patch, revision] = self.firmware;
        DeviceInfo {
            device_type: format!("{:04X}", self.wire.kind),
            device_id: format!("{:08X}", self.wire.number),
            firmware: format!("{major}.{minor}.{patch}.{revision}"),
        }
    }
}

#[derive(Serialize)]
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

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("SC9300 connection unavailable")]
    Unavailable,
    #[error("SC9300 read timed out")]
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
            Duration::from_secs(2)
        } else {
            Duration::from_millis(700)
        };
        let [kind1, kind2, id1, id2, id3, id4, major, minor, patch, revision] =
            self.exchange(target, READ_INFO, limit).await?;
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
            .exchange(identity.wire, STATUS, Duration::from_secs(2))
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

    async fn exchange<const N: usize>(
        &mut self,
        target: WireIdentity,
        command: u16,
        deadline: Duration,
    ) -> Result<[u8; N], Error> {
        self.pending.clear();
        let request = request(target, self.sequence, command);
        self.sequence = self.sequence.wrapping_add(1);
        timeout(deadline, async {
            self.link
                .write_all(&request)
                .await
                .map_err(|_| Error::Unavailable)?;
            let mut scratch = [0; 512];
            loop {
                if let Some(reply) = take_reply(&mut self.pending, target, command | 0x8000) {
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
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

fn request(target: WireIdentity, sequence: u32, command: u16) -> [u8; OVERHEAD] {
    let mut frame = [0; OVERHEAD];
    frame[..2].copy_from_slice(&PREAMBLE);
    frame[4..6].copy_from_slice(&target.kind.to_be_bytes());
    frame[6..10].copy_from_slice(&target.number.to_be_bytes());
    frame[10] = ADDRESS;
    frame[11..15].copy_from_slice(&sequence.to_be_bytes());
    frame[15..17].copy_from_slice(&command.to_be_bytes());
    frame[17] = checksum(&frame[..17]);
    frame[18..].copy_from_slice(&FOOTER);
    frame
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
