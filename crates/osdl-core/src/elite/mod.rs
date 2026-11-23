//! Native observation of Elite ES66 robot controllers.

mod state;
mod wire;

use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{sync::Mutex, task::JoinSet, time};

use crate::{
    adapter::{DeviceMatch, ProtocolAdapter},
    protocol::{ActionSchema, Device},
    transport::Transport,
    CommandResult, CommandStatus, DeviceCommand, EngineHandle,
};

const ADAPTER: &str = "elite";
const STATUS: &str = "status";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);
const READ_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StatusRequest {
    command_id: String,
    device_id: String,
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

    fn encode_command(&self, _device_type: &str, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        if cmd.action != STATUS {
            return Err("Elite ES66 supports only the status action".into());
        }
        if !cmd.params.is_null()
            && !cmd
                .params
                .as_object()
                .is_some_and(|params| params.is_empty())
        {
            return Err("Elite ES66 status does not accept arguments".into());
        }
        serde_json::to_vec(&StatusRequest {
            command_id: cmd.command_id.clone(),
            device_id: cmd.device_id.clone(),
        })
        .map_err(|_| "could not encode Elite ES66 status request".into())
    }

    fn decode_response(&self, _device_type: &str, _bytes: &[u8]) -> Option<HashMap<String, Value>> {
        None
    }
}

struct Robot {
    id: String,
    transport_id: String,
    host: Ipv4Addr,
    stopped: Mutex<bool>,
    connected: AtomicBool,
    engine: Weak<EngineHandle>,
}

impl Robot {
    fn device(&self) -> Device {
        Device {
            id: self.id.clone(),
            transport_id: self.transport_id.clone(),
            device_type: "elite.es66".into(),
            adapter: ADAPTER.into(),
            description: "Elite ES66 robot arm".into(),
            online: false,
            properties: HashMap::new(),
            properties_timestamp_ms: 0,
            actions: vec![ActionSchema {
                name: STATUS.into(),
                description: "Read the robot and safety state, base-frame TCP pose, and six joint angles from a single controller sample".into(),
                params: json!({"type":"object","properties":{},"additionalProperties":false}),
            }],
            role: None,
        }
    }

    async fn sample(&self) -> Result<wire::Snapshot, String> {
        let deadline = time::Instant::now() + READ_DEADLINE;
        let stopped = time::timeout_at(deadline, self.stopped.lock())
            .await
            .map_err(|_| "Elite ES66 controller is busy")?;
        if *stopped {
            return Err("Elite ES66 transport stopped".into());
        }
        let result = match time::timeout_at(deadline, wire::sample(self.host)).await {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err("Elite ES66 controller did not return a complete sample in time".into()),
        };
        self.record(result.as_ref().ok()).await;
        result
    }

    async fn record(&self, snapshot: Option<&wire::Snapshot>) {
        self.connected.store(snapshot.is_some(), Ordering::Relaxed);
        if let Some(engine) = self.engine.upgrade() {
            // Preserve the stable operating state in inventory. Pose and joint
            // angles are fresh in explicit action results and need no background history.
            let properties = snapshot.map(|sample| {
                HashMap::from([
                    ("identity".into(), json!(sample.identity)),
                    ("state".into(), json!(sample.state)),
                ])
            });
            engine
                .record_device_observation(&self.id, &self.transport_id, properties)
                .await;
        }
    }

    async fn monitor(&self) {
        let mut ticker = time::interval(OBSERVATION_INTERVAL);
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = self.sample().await {
                log::debug!("Elite ES66 {}: {error}", self.id);
            }
        }
    }
}

#[async_trait::async_trait]
impl Transport for Robot {
    fn transport_type(&self) -> &str {
        ADAPTER
    }

    fn description(&self) -> String {
        "Elite ES66 controller reader".into()
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        Err("Elite ES66 accepts only typed status requests".into())
    }

    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        let request: StatusRequest =
            serde_json::from_slice(bytes).map_err(|_| "invalid Elite ES66 status request")?;
        if request.device_id != self.id {
            return Err("Elite ES66 status request targets another device".into());
        }
        let sample = self.sample().await?;
        Ok(Some(CommandResult {
            command_id: request.command_id,
            device_id: request.device_id,
            status: CommandStatus::Succeeded,
            message: "Elite ES66 status read completed".into(),
            data: Some(json!(sample)),
        }))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn stop(&self) -> Result<(), String> {
        let mut stopped = self.stopped.lock().await;
        *stopped = true;
        self.record(None).await;
        Ok(())
    }
}

pub(crate) struct Runtime {
    monitors: JoinSet<()>,
    robots: Vec<Arc<Robot>>,
    _engine: Arc<EngineHandle>,
}

impl Runtime {
    pub(crate) async fn start(engine: &EngineHandle) -> Result<Option<Self>, String> {
        if engine.config().elite_robots.is_empty() {
            return Ok(None);
        }
        let engine = Arc::new(engine.clone());
        let mut ids = HashSet::new();
        let mut hosts = HashSet::new();
        let mut robots = Vec::new();
        for config in &engine.config().elite_robots {
            if config.id.is_empty()
                || config.id.len() > 128
                || !config
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            {
                return Err("invalid Elite ES66 device ID".into());
            }
            if !config.host.is_private() && !config.host.is_loopback() {
                return Err(
                    "Elite ES66 host must be a private IPv4 or local simulator address".into(),
                );
            }
            if !ids.insert(&config.id)
                || !hosts.insert(config.host)
                || engine.get_device(&config.id).await.is_some()
            {
                return Err("Elite ES66 device IDs and hosts must be unique".into());
            }
            robots.push(Arc::new(Robot {
                id: config.id.clone(),
                transport_id: format!("{ADAPTER}:{}", config.id),
                host: config.host,
                stopped: Mutex::default(),
                connected: AtomicBool::new(false),
                engine: Arc::downgrade(&engine),
            }));
        }

        for robot in &robots {
            engine
                .register_transport(robot.transport_id.clone(), robot.clone())
                .await;
            engine.register_device(robot.device()).await?;
        }
        let mut monitors = JoinSet::new();
        for robot in &robots {
            let robot = robot.clone();
            monitors.spawn(async move { robot.monitor().await });
        }
        Ok(Some(Self {
            monitors,
            robots,
            _engine: engine,
        }))
    }

    pub(crate) async fn stop(mut self) {
        self.monitors.shutdown().await;
        for robot in self.robots {
            let _ = robot.stop().await;
        }
    }
}
