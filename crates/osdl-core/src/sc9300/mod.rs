use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{self, MissedTickBehavior};

use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::config::Sc9300Config;
use crate::protocol::{ActionSchema, CommandResult, CommandStatus, Device, DeviceCommand};
use crate::transport::Transport;
use crate::EngineHandle;

mod wire;

#[cfg(test)]
mod tests;

use wire::{Client, Error, Identity};

const ADAPTER: &str = "sc9300";
const DEVICE_TYPE: &str = "prcxi.sc9300";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    DeviceInfo,
    Status,
}

impl Operation {
    fn parse(command: &DeviceCommand) -> Result<Self, String> {
        if !command.params.is_null() && !command.params.as_object().is_some_and(|p| p.is_empty()) {
            return Err("SC9300 reads do not accept arguments".into());
        }
        match command.action.as_str() {
            "device_info" => Ok(Self::DeviceInfo),
            "status" => Ok(Self::Status),
            _ => Err("unsupported SC9300 read".into()),
        }
    }

    fn schema(self) -> ActionSchema {
        let (name, description) = match self {
            Self::DeviceInfo => ("device_info", "Read the workstation identity and firmware"),
            Self::Status => (
                "status",
                "Read the workstation execution state and fault code",
            ),
        };
        ActionSchema {
            name: name.into(),
            description: description.into(),
            params: json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCommand {
    command_id: String,
    device_id: String,
    operation: Operation,
}

pub(crate) struct Adapter;

impl ProtocolAdapter for Adapter {
    fn platform(&self) -> &str {
        ADAPTER
    }

    fn load_registry(&mut self, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn match_hardware(&self, _hardware_id: &str) -> Option<DeviceMatch> {
        None
    }

    fn encode_command(
        &self,
        _device_type: &str,
        command: &DeviceCommand,
    ) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&ReadCommand {
            command_id: command.command_id.clone(),
            device_id: command.device_id.clone(),
            operation: Operation::parse(command)?,
        })
        .map_err(|_| "could not encode SC9300 read".into())
    }

    fn decode_response(&self, _device_type: &str, _bytes: &[u8]) -> Option<HashMap<String, Value>> {
        None
    }
}

trait Link: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Link for T {}

struct Session {
    client: Client<Box<dyn Link>>,
    identity: Identity,
}

impl Session {
    async fn open(port: &str) -> Result<Self, Error> {
        let mut client = Client::new(open_port(port)?);
        let discovered = client.identify(None).await?;
        let identity = client.identify(Some(discovered)).await?;
        Ok(Self { client, identity })
    }
}

#[derive(Default)]
struct State {
    session: Option<Session>,
    stopped: bool,
}

struct Station {
    id: String,
    transport_id: String,
    port: String,
    state: Mutex<State>,
    connected: AtomicBool,
    engine: Weak<EngineHandle>,
}

impl Station {
    fn new(config: &Sc9300Config, port: String, engine: &Arc<EngineHandle>) -> Self {
        Self {
            id: config.id.clone(),
            transport_id: format!("{ADAPTER}:{}", config.id),
            port,
            state: Mutex::new(State::default()),
            connected: AtomicBool::new(false),
            engine: Arc::downgrade(engine),
        }
    }

    fn device(&self) -> Device {
        Device {
            id: self.id.clone(),
            transport_id: self.transport_id.clone(),
            device_type: DEVICE_TYPE.into(),
            adapter: ADAPTER.into(),
            description: "PRCXI SC9300 workstation".into(),
            online: false,
            properties: HashMap::new(),
            properties_timestamp_ms: 0,
            actions: [Operation::DeviceInfo, Operation::Status]
                .into_iter()
                .map(Operation::schema)
                .collect(),
            role: None,
        }
    }

    async fn read(&self, operation: Operation) -> Result<Value, Error> {
        let mut state = self.state.lock().await;
        if state.stopped {
            return Err(Error::Unavailable);
        }
        // Removing the session before awaiting I/O also closes it on cancellation.
        // A later request then starts by identifying the serial peer again.
        let mut session = state.session.take();
        let result = self.sample(operation, &mut session).await;
        match result {
            Ok((value, properties)) => {
                state.session = session;
                if let Some(properties) = properties {
                    self.record(Some(properties)).await;
                }
                Ok(value)
            }
            Err(error) => {
                self.record(None).await;
                Err(error)
            }
        }
    }

    async fn sample(
        &self,
        operation: Operation,
        session: &mut Option<Session>,
    ) -> Result<(Value, Option<HashMap<String, Value>>), Error> {
        let already_open = session.is_some();
        let session = match session {
            Some(session) => session,
            None => session.insert(Session::open(&self.port).await?),
        };
        match operation {
            Operation::DeviceInfo => {
                if already_open {
                    session.identity = session.client.identify(Some(session.identity)).await?;
                }
                let identity = json!(session.identity.info());
                // An identity-only read on the same connection cannot renew
                // the timestamp of a previously sampled status or erase it.
                let properties =
                    (!already_open).then(|| HashMap::from([("identity".into(), identity.clone())]));
                Ok((identity, properties))
            }
            Operation::Status => {
                let status = json!(session.client.status(session.identity).await?);
                let properties = HashMap::from([
                    ("identity".into(), json!(session.identity.info())),
                    ("status".into(), status.clone()),
                ]);
                Ok((status, Some(properties)))
            }
        }
    }

    async fn monitor(&self) {
        let mut observations = time::interval(OBSERVATION_INTERVAL);
        observations.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            observations.tick().await;
            if let Err(error) = self.read(Operation::Status).await {
                log::debug!("SC9300 workstation {}: {error}", self.id);
            }
        }
    }

    async fn record(&self, properties: Option<HashMap<String, Value>>) {
        self.connected
            .store(properties.is_some(), Ordering::Relaxed);
        if let Some(engine) = self.engine.upgrade() {
            engine
                .record_device_observation(&self.id, &self.transport_id, properties)
                .await;
        }
    }

    async fn shutdown(&self) {
        let mut state = self.state.lock().await;
        state.stopped = true;
        state.session = None;
        self.record(None).await;
    }
}

#[async_trait::async_trait]
impl Transport for Station {
    fn transport_type(&self) -> &str {
        ADAPTER
    }

    fn description(&self) -> String {
        "PRCXI SC9300 serial reader".into()
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        Err("SC9300 accepts only typed reads".into())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        let request: ReadCommand =
            serde_json::from_slice(bytes).map_err(|_| "invalid SC9300 read")?;
        if request.device_id != self.id {
            return Err("SC9300 read targets another device".into());
        }
        let data = time::timeout(REQUEST_DEADLINE, self.read(request.operation))
            .await
            .map_err(|_| Error::Timeout.to_string())?
            .map_err(|error| error.to_string())?;
        Ok(Some(CommandResult {
            device_id: request.device_id,
            command_id: request.command_id,
            status: CommandStatus::Succeeded,
            message: "SC9300 read completed".into(),
            data: Some(data),
        }))
    }
}

pub(crate) struct Runtime {
    monitors: JoinSet<()>,
    stations: Vec<Arc<Station>>,
    // Stations refer back weakly because the engine also owns their transports.
    _engine: Arc<EngineHandle>,
}

impl Runtime {
    pub(crate) async fn start(engine: &EngineHandle) -> Result<Option<Self>, String> {
        if engine.config().sc9300_instruments.is_empty() {
            return Ok(None);
        }
        let engine = Arc::new(engine.clone());
        let mut ids = HashSet::new();
        let mut ports: HashSet<_> = engine
            .config()
            .espnow_dongles
            .iter()
            .filter_map(|dongle| port_name(&dongle.port))
            .map(|name| port_key(&name))
            .collect();
        let mut stations = Vec::new();
        for config in &engine.config().sc9300_instruments {
            let port = port_name(&config.port).ok_or("SC9300 requires a local serial port")?;
            if config.id.is_empty()
                || config.id.len() > 128
                || !config
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            {
                return Err("invalid SC9300 workstation ID".into());
            }
            if !ids.insert(&config.id) || !ports.insert(port_key(&port)) {
                return Err("SC9300 workstation IDs and serial ports must be unique".into());
            }
            if engine.get_device(&config.id).await.is_some() {
                return Err("SC9300 workstation ID is already registered".into());
            }
            stations.push(Arc::new(Station::new(config, port, &engine)));
        }

        let mut monitors = JoinSet::new();
        for station in &stations {
            engine
                .register_transport(station.transport_id.clone(), station.clone())
                .await;
            engine.register_device(station.device()).await?;
            let station = station.clone();
            monitors.spawn(async move { station.monitor().await });
        }
        Ok(Some(Self {
            monitors,
            stations,
            _engine: engine,
        }))
    }

    pub(crate) async fn stop(mut self) {
        self.monitors.shutdown().await;
        for station in self.stations {
            station.shutdown().await;
        }
    }
}

fn port_name(input: &str) -> Option<String> {
    if cfg!(windows) {
        let name = input
            .strip_prefix(r"\\.\")
            .unwrap_or(input)
            .to_ascii_uppercase();
        let number = name.strip_prefix("COM")?;
        if !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let number = number.parse::<u16>().ok()?;
        return (number != 0).then(|| format!("COM{number}"));
    }
    Path::new(input).is_absolute().then(|| input.to_owned())
}

fn port_key(name: &str) -> String {
    if cfg!(windows) {
        return name.to_owned();
    }
    std::fs::canonicalize(name)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| name.to_owned())
}

#[cfg(any(feature = "serial", feature = "espnow"))]
fn open_port(name: &str) -> Result<Box<dyn Link>, Error> {
    use tokio_serial::{DataBits, FlowControl, Parity, SerialPortBuilderExt, StopBits};

    let port = tokio_serial::new(name, 115_200)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .open_native_async()
        .map_err(|_| Error::Unavailable)?;
    Ok(Box::new(port))
}

#[cfg(not(any(feature = "serial", feature = "espnow")))]
fn open_port(_name: &str) -> Result<Box<dyn Link>, Error> {
    Err(Error::Unavailable)
}
