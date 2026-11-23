//! Dashboard and RTSI v1 output protocol from the bundled Elite CS SDK.

use std::{io, net::Ipv4Addr};

use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

use super::state::RobotState;

const DASHBOARD_PORT: u16 = 29999;
const RTSI_PORT: u16 = 30004;
const MODEL: &str = "ES66";
const RECIPE: &[u8] = b"timestamp,actual_TCP_pose,actual_joint_positions,robot_mode,joint_mode,safety_status,runtime_state";
const RECIPE_TYPES: &[u8] = b"DOUBLE,VECTOR6D,VECTOR6D,INT32,VECTOR6INT32,INT32,UINT32";
// The vendor recorder cadence reduces time to the first sample. The socket
// closes as soon as that sample arrives.
const SAMPLE_RATE_HZ: f64 = 125.0;
const MAX_FRAME: usize = 1024;
const SAMPLE_BYTES: usize = 8 + 2 * 6 * 8 + 9 * 4;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("Elite controller connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("Elite Dashboard did not identify the controller as ES66")]
    Identity,
    #[error("Elite RTSI {0}")]
    Rtsi(&'static str),
}

#[derive(Serialize)]
pub(super) struct Identity {
    model: &'static str,
    controller_version: String,
}

#[derive(Serialize)]
pub(super) struct TcpPose {
    frame: &'static str,
    pub(super) position_m: [f64; 3],
    pub(super) rotation_vector_rad: [f64; 3],
}

#[derive(Serialize)]
pub(super) struct Snapshot {
    pub(super) identity: Identity,
    pub(super) state: RobotState,
    pub(super) controller_timestamp_s: f64,
    pub(super) tcp_pose: TcpPose,
    joint_positions_rad: [f64; 6],
}

pub(super) async fn sample(host: Ipv4Addr) -> Result<Snapshot, Error> {
    let (_, sample) = tokio::try_join!(identify(host), rtsi_sample(host))?;
    Ok(sample)
}

async fn identify(host: Ipv4Addr) -> Result<(), Error> {
    if dashboard_request(host, b"robot -t\n").await? != MODEL {
        return Err(Error::Identity);
    }
    Ok(())
}

pub(super) async fn loaded_task(host: Ipv4Addr) -> Result<String, Error> {
    dashboard_request(host, b"task\n").await
}

async fn dashboard_request(host: Ipv4Addr, request: &[u8]) -> Result<String, Error> {
    let mut socket = BufReader::new(TcpStream::connect((host, DASHBOARD_PORT)).await?);
    if !dashboard_line(&mut socket)
        .await?
        .starts_with("EliRobot dashboard shell, version ")
    {
        return Err(Error::Identity);
    }
    socket.write_all(request).await?;
    for _ in 0..4 {
        let line = dashboard_line(&mut socket).await?;
        if !["These shell commands ", "Type `help ", "Type `usage "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            return Ok(line);
        }
    }
    Err(Error::Identity)
}

async fn dashboard_line(socket: &mut BufReader<TcpStream>) -> Result<String, Error> {
    let mut line = String::new();
    socket.take(512).read_line(&mut line).await?;
    let length = line.trim_end_matches(['\r', '\n']).len();
    if !line.ends_with('\n') {
        return Err(Error::Identity);
    }
    line.truncate(length);
    Ok(line)
}

async fn rtsi_sample(host: Ipv4Addr) -> Result<Snapshot, Error> {
    let mut socket = TcpStream::connect((host, RTSI_PORT)).await?;
    socket.set_nodelay(true)?;
    let mut buffer = [0; MAX_FRAME];

    send(&mut socket, b'V', &1u16.to_be_bytes()).await?;
    if receive(&mut socket, b'V', &mut buffer).await? != [1] {
        return Err(Error::Rtsi("controller refused protocol version 1"));
    }
    send(&mut socket, b'v', &[]).await?;
    let version = receive(&mut socket, b'v', &mut buffer).await?;
    if version.len() != 16 {
        return Err(Error::Rtsi("invalid controller version frame"));
    }
    let [major, minor, bugfix, build] = ints(version);
    let identity = Identity {
        model: MODEL,
        controller_version: format!("{major}.{minor}.{bugfix}.{build}"),
    };

    let mut recipe = [0; 8 + RECIPE.len()];
    recipe[..8].copy_from_slice(&SAMPLE_RATE_HZ.to_be_bytes());
    recipe[8..].copy_from_slice(RECIPE);
    send(&mut socket, b'O', &recipe).await?;
    let recipe_id = match receive(&mut socket, b'O', &mut buffer).await? {
        [recipe_id, types @ ..] if types == RECIPE_TYPES => *recipe_id,
        _ => {
            return Err(Error::Rtsi(
                "controller returned an unexpected output recipe",
            ))
        }
    };

    send(&mut socket, b'S', &[]).await?;
    if receive(&mut socket, b'S', &mut buffer).await? != [1] {
        return Err(Error::Rtsi("controller refused to start the output stream"));
    }
    let payload = receive(&mut socket, b'U', &mut buffer).await?;
    let [id, sample @ ..] = payload else {
        return Err(Error::Rtsi("missing recipe ID in sample"));
    };
    if *id != recipe_id || sample.len() != SAMPLE_BYTES {
        return Err(Error::Rtsi("sample does not match the output recipe"));
    }
    let coordinates = doubles::<13>(&sample[..13 * 8]);
    if coordinates.iter().any(|value| !value.is_finite()) || coordinates[0] < 0.0 {
        return Err(Error::Rtsi("invalid joint, pose, or timestamp measurement"));
    }
    let [timestamp, x, y, z, rx, ry, rz, j1, j2, j3, j4, j5, j6] = coordinates;
    Ok(Snapshot {
        identity,
        state: RobotState::from_codes(ints(&sample[13 * 8..])),
        controller_timestamp_s: timestamp,
        tcp_pose: TcpPose {
            frame: "robot_base",
            position_m: [x, y, z],
            rotation_vector_rad: [rx, ry, rz],
        },
        joint_positions_rad: [j1, j2, j3, j4, j5, j6],
    })
}

fn ints<const N: usize>(bytes: &[u8]) -> [i32; N] {
    std::array::from_fn(|i| i32::from_be_bytes(bytes[i * 4..][..4].try_into().unwrap()))
}

fn doubles<const N: usize>(bytes: &[u8]) -> [f64; N] {
    std::array::from_fn(|i| f64::from_be_bytes(bytes[i * 8..][..8].try_into().unwrap()))
}

async fn send(socket: &mut TcpStream, kind: u8, payload: &[u8]) -> Result<(), Error> {
    let mut frame = [0; 3 + 8 + RECIPE.len()];
    let size = 3 + payload.len();
    frame[..2].copy_from_slice(&(size as u16).to_be_bytes());
    frame[2] = kind;
    frame[3..size].copy_from_slice(payload);
    socket.write_all(&frame[..size]).await?;
    Ok(())
}

async fn receive<'a>(
    socket: &mut TcpStream,
    expected: u8,
    buffer: &'a mut [u8; MAX_FRAME],
) -> Result<&'a [u8], Error> {
    loop {
        let mut header = [0; 3];
        socket.read_exact(&mut header).await?;
        let size = usize::from(u16::from_be_bytes([header[0], header[1]]));
        if !(3..=3 + MAX_FRAME).contains(&size) {
            return Err(Error::Rtsi("received an invalid frame length"));
        }
        let payload = &mut buffer[..size - 3];
        socket.read_exact(payload).await?;
        match header[2] {
            kind if kind == expected => return Ok(&buffer[..size - 3]),
            b'M' => continue, // Unsolicited controller text packet.
            _ => return Err(Error::Rtsi("received an unexpected frame type")),
        }
    }
}
