use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::JoinSet;
use tokio::time::{self, MissedTickBehavior};

use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::config::Qx96mConfig;
use crate::protocol::{ActionSchema, CommandResult, CommandStatus, Device, DeviceCommand};
use crate::transport::Transport;
use crate::EngineHandle;

use super::client::{Client, EquipmentState, Identity};

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
    client: Client,
    online: AtomicBool,
}

impl Station {
    fn new(config: &Qx96mConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            id: config.id.clone(),
            transport_id: format!("{ADAPTER}:{}", config.id),
            client: Client::new(config.endpoint, load_key(&config.key_file)?),
            online: AtomicBool::new(false),
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

    async fn sample(
        &self,
        read_status: bool,
    ) -> Result<(Identity, Option<EquipmentState>), String> {
        let result = self.client.read(read_status).await;
        self.online.store(result.is_ok(), Ordering::Relaxed);
        result.map_err(|error| error.to_string())
    }

    async fn monitor(&self, engine: EngineHandle) {
        let mut observations = time::interval(OBSERVATION_INTERVAL);
        observations.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            observations.tick().await;
            let properties = match self.sample(true).await {
                Ok((identity, state)) => Some(HashMap::from([
                    ("identity".into(), json!(identity)),
                    ("state".into(), json!(state)),
                ])),
                Err(error) => {
                    log::debug!("QX96M device {}: {error}", self.id);
                    None
                }
            };
            engine
                .record_device_observation(&self.id, &self.transport_id, properties)
                .await;
        }
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
        let read_status = matches!(command.action, Action::Status);
        let (identity, state) = self.sample(read_status).await?;
        let data = match state {
            Some(state) => json!({"state":state}),
            None => json!(identity),
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
}

impl Runtime {
    pub(crate) async fn start(engine: &EngineHandle) -> Result<Self, String> {
        let mut ids = HashSet::new();
        let mut endpoints = HashSet::new();
        let mut stations = Vec::new();
        for config in &engine.config().qx96m_instruments {
            if !ids.insert(&config.id) || !endpoints.insert(config.endpoint) {
                return Err("QX96M device IDs and endpoints must be unique".into());
            }
            stations.push(Arc::new(Station::new(config)?));
        }
        let mut monitors = JoinSet::new();
        for station in &stations {
            engine
                .register_transport(station.transport_id.clone(), station.clone())
                .await;
            engine.register_device(station.device()).await?;
            let station = station.clone();
            let engine = engine.clone();
            monitors.spawn(async move { station.monitor(engine).await });
        }
        Ok(Self { monitors, stations })
    }

    pub(crate) async fn stop(mut self, engine: &EngineHandle) {
        self.monitors.shutdown().await;
        for station in self.stations {
            station.client.stop().await;
            station.online.store(false, Ordering::Relaxed);
            engine
                .record_device_observation(&station.id, &station.transport_id, None)
                .await;
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
