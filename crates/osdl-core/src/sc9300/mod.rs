use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{self, MissedTickBehavior};

use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::config::Sc9300Config;
use crate::engine::now_millis;
use crate::protocol::{observed_command_data, CommandResult, CommandStatus, Device, DeviceCommand};
use crate::transport::Transport;
use crate::EngineHandle;

mod actions;
mod calibration;
mod wire;

#[cfg(test)]
mod tests;

use actions::{Axis, Operation, Request};
use wire::{Client, Command, Dispatch, Error, Identity, Position, Status};

const ADAPTER: &str = "sc9300";
const DEVICE_TYPE: &str = "prcxi.sc9300";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(120);
const EXCHANGE_BUDGET: Duration = Duration::from_secs(5);
const TELEMETRY_BUDGET: Duration = Duration::from_secs(12);

enum Reply {
    Identity(Identity),
    Status(Status),
    Position(Position),
    Telemetry(Status, Vec<Position>),
    Dispatched(Dispatch),
    Limit(Dispatch),
}

impl Reply {
    fn outcome(self, device_id: &str) -> (CommandStatus, &'static str, Value) {
        let (data, properties) = match self {
            Self::Identity(identity) => (identity.info().command_data(), identity.observation()),
            Self::Status(status) => (json!(status), status.observation()),
            Self::Position(position) => (json!(position), position.observation().collect()),
            Self::Telemetry(status, heads) => {
                let mut properties = status.observation();
                properties.extend(heads.iter().flat_map(Position::observation));
                (json!({ "status": status, "heads": heads }), properties)
            }
            Self::Dispatched(dispatch) => {
                return (
                    CommandStatus::Pending,
                    "SC9300 command sent; check status or wait_idle for the controller state",
                    json!(dispatch),
                );
            }
            Self::Limit(dispatch) => {
                return (
                    CommandStatus::Pending,
                    "SC9300 limit probe sent; the controller did not confirm reachability",
                    json!({ "reachable": Value::Null, "dispatch": dispatch }),
                );
            }
        };
        (
            CommandStatus::Succeeded,
            "SC9300 read completed",
            observed_command_data(device_id, now_millis(), properties, data),
        )
    }
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
        serde_json::to_vec(command).map_err(|_| "could not encode SC9300 action".into())
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

    async fn execute(
        &mut self,
        operation: Operation,
        already_open: bool,
        deadline: time::Instant,
    ) -> Result<Reply, Error> {
        match operation {
            Operation::DeviceInfo => {
                if already_open {
                    self.identity = self.client.identify(Some(self.identity)).await?;
                }
                Ok(Reply::Identity(self.identity))
            }
            Operation::Status => Ok(Reply::Status(self.client.status(self.identity).await?)),
            Operation::Position(axis) => Ok(Reply::Position(
                self.client.position(self.identity, axis).await?,
            )),
            Operation::Telemetry => {
                let status = self.client.status(self.identity).await?;
                let mut heads = Vec::with_capacity(Axis::ALL.len());
                for axis in Axis::ALL {
                    heads.push(self.client.position(self.identity, axis).await?);
                }
                Ok(Reply::Telemetry(status, heads))
            }
            Operation::CheckLimit(payload) => {
                let dispatch = self
                    .client
                    .dispatch(self.identity, Command::CheckLimit, &payload, deadline)
                    .await?;
                Ok(Reply::Limit(dispatch))
            }
            Operation::Dispatch(command, payload) => {
                let dispatch = self
                    .client
                    .dispatch(self.identity, command, &payload, deadline)
                    .await?;
                Ok(Reply::Dispatched(dispatch))
            }
        }
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
            actions: actions::schemas(),
            role: None,
        }
    }

    async fn exchange(
        &self,
        operation: Operation,
        deadline: time::Instant,
    ) -> Result<Reply, Error> {
        let mut state = time::timeout_at(deadline, self.state.lock())
            .await
            .map_err(|_| Error::Timeout)?;
        if state.stopped {
            return Err(Error::Unavailable);
        }
        // Do not start a new frame once a queued request has expired.
        if time::Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        // Removing the session before awaiting I/O closes it on cancellation.
        let previous = state.session.take();
        let already_open = previous.is_some();
        let dispatch = matches!(
            operation,
            Operation::Dispatch(..) | Operation::CheckLimit(_)
        );
        let result = async {
            let mut session = match previous {
                Some(session) => session,
                None => time::timeout_at(deadline, Session::open(&self.port))
                    .await
                    .map_err(|_| Error::Timeout)??,
            };
            let exchange = session.execute(operation, already_open, deadline);
            // Dispatch bounds its write and response independently so a timeout
            // observing the reply cannot discard a command already on the wire.
            let reply = if dispatch {
                exchange.await?
            } else {
                time::timeout_at(deadline, exchange)
                    .await
                    .map_err(|_| Error::Timeout)??
            };
            Ok((session, reply))
        }
        .await;
        match result {
            Ok((session, reply)) => {
                if matches!(&reply, Reply::Dispatched(dispatch) | Reply::Limit(dispatch) if dispatch.connection_lost())
                {
                    drop(session);
                    self.record(None).await;
                    return Ok(reply);
                }
                let status = match &reply {
                    Reply::Status(status) | Reply::Telemetry(status, _) => Some(status),
                    _ => None,
                };
                if status.is_some() || !already_open {
                    let mut properties =
                        HashMap::from([("identity".into(), json!(session.identity.info()))]);
                    if let Some(status) = status {
                        properties.insert("status".into(), json!(status));
                    }
                    self.record(Some(properties)).await;
                }
                state.session = Some(session);
                Ok(reply)
            }
            Err(error) => {
                self.record(None).await;
                Err(error)
            }
        }
    }

    async fn wait_idle(
        &self,
        budget: Duration,
    ) -> Result<(CommandStatus, &'static str, Value), Error> {
        let deadline = time::Instant::now() + budget;
        let mut latest = None;
        let (outcome, message, status) = loop {
            match self.exchange(Operation::Status, deadline).await {
                Ok(Reply::Status(status)) if status.idle() => {
                    break (
                        CommandStatus::Succeeded,
                        "SC9300 controller is idle",
                        status,
                    );
                }
                Ok(Reply::Status(status)) if status.faulted() => {
                    break (
                        CommandStatus::Failed,
                        "SC9300 controller reported a fault",
                        status,
                    );
                }
                Ok(Reply::Status(status)) => latest = Some(status),
                Err(Error::Timeout) if time::Instant::now() >= deadline => {
                    break (
                        CommandStatus::Running,
                        "SC9300 controller has not settled",
                        latest.ok_or(Error::Timeout)?,
                    );
                }
                Err(error) => return Err(error),
                Ok(_) => unreachable!("status operation returns status"),
            }
            time::sleep_until((time::Instant::now() + STATUS_POLL_INTERVAL).min(deadline)).await;
        };
        let data = json!({ "settled": status.idle(), "status": status });
        let data = if outcome == CommandStatus::Succeeded {
            observed_command_data(&self.id, now_millis(), status.observation(), data)
        } else {
            data
        };
        Ok((outcome, message, data))
    }

    async fn monitor(&self) {
        let mut observations = time::interval(OBSERVATION_INTERVAL);
        observations.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            observations.tick().await;
            if let Err(error) = self
                .exchange(Operation::Status, time::Instant::now() + EXCHANGE_BUDGET)
                .await
            {
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
        "PRCXI SC9300 serial instrument".into()
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        Err("SC9300 accepts only typed actions".into())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        let command: DeviceCommand =
            serde_json::from_slice(bytes).map_err(|_| "invalid SC9300 action")?;
        if command.device_id != self.id {
            return Err("SC9300 action targets another device".into());
        }
        let (status, message, data) = match Request::parse(&command)? {
            Request::Exchange(operation) => {
                let budget = if matches!(operation, Operation::Telemetry) {
                    TELEMETRY_BUDGET
                } else {
                    EXCHANGE_BUDGET
                };
                self.exchange(operation, time::Instant::now() + budget)
                    .await
                    .map(|reply| reply.outcome(&self.id))
            }
            Request::WaitIdle(budget) => self.wait_idle(budget).await,
        }
        .map_err(|error| error.to_string())?;
        Ok(Some(CommandResult {
            device_id: command.device_id,
            command_id: command.command_id,
            status,
            message: message.into(),
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
