use std::net::SocketAddrV4;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::time::{self, Instant};

use super::{wire, Error};

const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(
    DIAL_TIMEOUT.as_secs() + CONNECT_TIMEOUT.as_secs() + STATUS_TIMEOUT.as_secs(),
);
const SESSION_LIFETIME: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
pub(super) struct Identity {
    model: &'static str,
    device_name: Option<String>,
    serial_number: String,
    firmware: Option<String>,
    rows: u16,
    columns: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EquipmentState {
    Idle,
    Active,
}

pub(super) struct Observation {
    pub identity: Identity,
    pub state: Option<EquipmentState>,
}

pub(super) struct Client {
    endpoint: SocketAddrV4,
    key: [u8; 8],
    next_id: u64,
    session: Option<Session>,
}

struct Session {
    link: BufReader<TcpStream>,
    expires: Instant,
    identity: Identity,
    state: Option<EquipmentState>,
}

impl Client {
    pub fn new(endpoint: SocketAddrV4, key: [u8; 8]) -> Self {
        Self {
            endpoint,
            key,
            next_id: 0,
            session: None,
        }
    }

    pub async fn read(&mut self, status: bool) -> Result<Observation, Error> {
        // A cancelled or malformed exchange drops the socket with its in-flight
        // reply. A subsequent read gets a fresh session.
        let minimum_lifetime = if status {
            STATUS_TIMEOUT
        } else {
            Duration::ZERO
        };
        let mut session = match self.session.take() {
            Some(session) if session.expires > Instant::now() + minimum_lifetime => session,
            _ => {
                let id = self.request_id();
                self.connect(id).await?
            }
        };
        if status {
            let deadline = session.expires.min(Instant::now() + STATUS_TIMEOUT);
            let id = self.request_id();
            let reply = wire::exchange(
                &mut session.link,
                &self.key,
                id,
                wire::Read::Status,
                deadline,
            )
            .await?;
            session.state = Some(project_state(&reply)?);
        }
        let observation = Observation {
            identity: session.identity.clone(),
            state: session.state,
        };
        self.session = Some(session);
        Ok(observation)
    }

    pub fn close(&mut self) {
        self.session = None;
    }

    async fn connect(&self, id: u64) -> Result<Session, Error> {
        let stream = time::timeout(DIAL_TIMEOUT, TcpStream::connect(self.endpoint))
            .await
            .map_err(|_| Error::Unavailable)??;
        let mut link = BufReader::new(stream);
        let expires = Instant::now() + SESSION_LIFETIME;
        let reply = wire::exchange(
            &mut link,
            &self.key,
            id,
            wire::Read::Connect,
            Instant::now() + CONNECT_TIMEOUT,
        )
        .await?;
        let identity = project_identity(&reply)?;
        Ok(Session {
            link,
            expires,
            identity,
            state: None,
        })
    }

    fn request_id(&mut self) -> u64 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }
}

fn project_identity(reply: &wire::Reply) -> Result<Identity, Error> {
    #[derive(Deserialize)]
    struct Station {
        #[serde(rename = "PCRName")]
        name: Option<String>,
        #[serde(rename = "SN")]
        serial: String,
        #[serde(rename = "SoftVersion")]
        firmware: Option<String>,
        #[serde(rename = "RowsCount")]
        rows: u16,
        #[serde(rename = "ColumnCount")]
        columns: u16,
    }

    if !reply.accepted {
        return Err(Error::Rejected);
    }
    let data = reply.data.as_deref().ok_or(Error::Protocol)?;
    let station: Station = serde_json::from_str(data).map_err(|_| Error::Protocol)?;
    if (station.rows, station.columns) != (8, 12) {
        return Err(Error::Protocol);
    }
    Ok(Identity {
        model: "QX96M",
        device_name: station.name.and_then(label),
        serial_number: label(station.serial).ok_or(Error::Protocol)?,
        firmware: station.firmware.and_then(label),
        rows: station.rows,
        columns: station.columns,
    })
}

fn project_state(reply: &wire::Reply) -> Result<EquipmentState, Error> {
    let data = reply.data.as_deref().ok_or(Error::Protocol)?;
    if !reply.accepted {
        return if data.trim() == "PCR仪器上无正在运行的实验!" {
            Ok(EquipmentState::Idle)
        } else {
            Err(Error::Rejected)
        };
    }
    #[derive(Deserialize)]
    struct ActiveState<'a> {
        #[serde(rename = "ID")]
        id: &'a str,
    }
    let state: ActiveState<'_> = serde_json::from_str(data).map_err(|_| Error::Protocol)?;
    if state.id.len() != 32 || !state.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Protocol);
    }
    Ok(EquipmentState::Active)
}

fn label(value: String) -> Option<String> {
    let label = value.trim();
    if label.is_empty()
        || label.chars().take(161).count() > 160
        || label.chars().any(char::is_control)
    {
        return None;
    }
    Some(label.to_owned())
}
