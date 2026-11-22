use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::{self, MissedTickBehavior};

use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::config::Qx96mConfig;
use crate::protocol::{ActionSchema, CommandResult, CommandStatus, Device, DeviceCommand};
use crate::transport::Transport;
use crate::EngineHandle;

use super::client::{Client, Observation, READ_TIMEOUT};
use super::Error;

const DEVICE_TYPE: &str = "jialeme.qpcr";
const ADAPTER: &str = "qx96m";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Identity,
    Status,
}

impl Action {
    const ALL: [Self; 2] = [Self::Identity, Self::Status];

    fn name(self) -> &'static str {
        match self {
            Self::Identity => "device_info",
            Self::Status => "run_status",
        }
    }

    fn schema(self) -> ActionSchema {
        let description = match self {
            Self::Identity => "Read the instrument identity",
            Self::Status => "Read whether the instrument is idle or active",
        };
        ActionSchema {
            name: self.name().into(),
            description: description.into(),
            params: json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }

    fn parse(command: &DeviceCommand) -> Result<Self, String> {
        if !command.params.is_null() && !command.params.as_object().is_some_and(|v| v.is_empty()) {
            return Err("QX96M reads do not accept arguments".into());
        }
        Self::ALL
            .into_iter()
            .find(|action| action.name() == command.action)
            .ok_or_else(|| "unsupported QX96M read".into())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCommand {
    device_id: String,
    command_id: String,
    action: Action,
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
        let action = Action::parse(command)?;
        serde_json::to_vec(&ReadCommand {
            device_id: command.device_id.clone(),
            command_id: command.command_id.clone(),
            action,
        })
        .map_err(|_| "could not encode QX96M read".into())
    }

    fn decode_response(&self, _device_type: &str, _bytes: &[u8]) -> Option<HashMap<String, Value>> {
        None
    }
}

struct Station {
    id: String,
    transport_id: String,
    reader: Mutex<Reader>,
    online: AtomicBool,
    engine: Weak<EngineHandle>,
}

struct Reader {
    client: Client,
    stopped: bool,
    last_failure: Option<Error>,
}

impl Station {
    fn new(config: &Qx96mConfig, engine: &Arc<EngineHandle>) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            id: config.id.clone(),
            transport_id: format!("{ADAPTER}:{}", config.id),
            reader: Mutex::new(Reader {
                client: Client::new(config.endpoint, load_key(&config.key_file)?),
                stopped: false,
                last_failure: None,
            }),
            online: AtomicBool::new(false),
            engine: Arc::downgrade(engine),
        })
    }

    fn device(&self) -> Device {
        Device {
            id: self.id.clone(),
            transport_id: self.transport_id.clone(),
            device_type: DEVICE_TYPE.into(),
            adapter: ADAPTER.into(),
            description: "JLM QX96M instrument".into(),
            online: false,
            properties: HashMap::new(),
            properties_timestamp_ms: 0,
            actions: Action::ALL.into_iter().map(Action::schema).collect(),
            role: None,
        }
    }

    async fn read(&self, action: Action) -> Result<Observation, Error> {
        // Bound admission and I/O independently, so time spent waiting for
        // another read never shortens a newly acquired instrument exchange.
        let mut reader = time::timeout(READ_TIMEOUT, self.reader.lock())
            .await
            .map_err(|_| Error::Unavailable)?;
        if reader.stopped {
            return Err(Error::Unavailable);
        }
        // The same owner serializes reads and publishes their observations in
        // order. A status from the monitor cannot overwrite a newer RPC read.
        let result = reader.client.read(matches!(action, Action::Status)).await;
        let properties = match &result {
            Ok(observation) => {
                reader.last_failure = None;
                let mut properties =
                    HashMap::from([("identity".into(), json!(observation.identity))]);
                if let Some(state) = observation.state {
                    properties.insert("state".into(), json!(state));
                }
                Some(properties)
            }
            Err(error) => {
                if reader.last_failure != Some(*error) {
                    log::warn!("QX96M device {}: {error}", self.id);
                    reader.last_failure = Some(*error);
                }
                None
            }
        };
        self.record(properties).await;
        result
    }

    async fn record(&self, properties: Option<HashMap<String, Value>>) {
        self.online.store(properties.is_some(), Ordering::Relaxed);
        if let Some(engine) = self.engine.upgrade() {
            engine
                .record_device_observation(&self.id, &self.transport_id, properties)
                .await;
        }
    }

    async fn monitor(&self) {
        let mut observations = time::interval(OBSERVATION_INTERVAL);
        observations.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            observations.tick().await;
            let _ = self.read(Action::Status).await;
        }
    }

    async fn stop(&self) {
        let mut reader = self.reader.lock().await;
        reader.stopped = true;
        reader.client.close();
        self.record(None).await;
    }
}

#[async_trait::async_trait]
impl Transport for Station {
    fn transport_type(&self) -> &str {
        ADAPTER
    }

    fn description(&self) -> String {
        "JLM QX96M local equipment reader".into()
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        Err("QX96M accepts only typed reads".into())
    }

    fn is_connected(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        let command: ReadCommand =
            serde_json::from_slice(bytes).map_err(|_| "invalid QX96M read")?;
        if command.device_id != self.id {
            return Err("QX96M read targets another device".into());
        }
        let observation = self
            .read(command.action)
            .await
            .map_err(|error| error.to_string())?;
        let data = match command.action {
            Action::Identity => json!(observation.identity),
            Action::Status => json!({"state":observation.state.ok_or("QX96M status unavailable")?}),
        };
        Ok(Some(CommandResult {
            device_id: command.device_id,
            command_id: command.command_id,
            status: CommandStatus::Succeeded,
            message: "QX96M equipment read completed".into(),
            data: Some(data),
        }))
    }
}

pub(crate) struct Runtime {
    monitors: JoinSet<()>,
    stations: Vec<Arc<Station>>,
    // Registered transports refer back weakly to avoid an ownership cycle.
    _engine: Arc<EngineHandle>,
}

impl Runtime {
    pub(crate) async fn start(engine: &EngineHandle) -> Result<Option<Self>, String> {
        if engine.config().qx96m_instruments.is_empty() {
            return Ok(None);
        }
        let engine = Arc::new(engine.clone());
        let mut ids = HashSet::new();
        let mut endpoints = HashSet::new();
        let mut stations = Vec::new();
        for config in &engine.config().qx96m_instruments {
            if !ids.insert(&config.id) || !endpoints.insert(config.endpoint) {
                return Err("QX96M device IDs and endpoints must be unique".into());
            }
            if engine.get_device(&config.id).await.is_some() {
                return Err(format!(
                    "QX96M device ID '{}' is already registered",
                    config.id
                ));
            }
            stations.push(Arc::new(Station::new(config, &engine)?));
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
            station.stop().await;
        }
    }
}

fn load_key(path: &Path) -> Result<[u8; 8], String> {
    let error = || "QX96M key_file must be a regular file with exactly eight printable ASCII bytes";
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(error().into());
    }
    let file = std::fs::File::open(path).map_err(|_| error())?;
    let mut bytes = Vec::with_capacity(9);
    file.take(9).read_to_end(&mut bytes).map_err(|_| error())?;
    if !bytes.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        return Err(error().into());
    }
    bytes.try_into().map_err(|_| error().into())
}

#[cfg(test)]
mod tests;
