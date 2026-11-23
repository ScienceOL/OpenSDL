//! Native telemetry and typed Cartesian control of Elite ES66 robot controllers.

mod motion;
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
    protocol::{observed_command_data, ActionSchema, Device},
    transport::Transport,
    CommandResult, CommandStatus, DeviceCommand, EngineHandle,
};

const ADAPTER: &str = "elite";
const STATUS: &str = "status";
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);
const READ_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    command_id: String,
    device_id: String,
    operation: Operation,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Status,
    MoveXyz(motion::Target),
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
        let operation = match cmd.action.as_str() {
            STATUS => {
                if !cmd.params.is_null()
                    && !cmd
                        .params
                        .as_object()
                        .is_some_and(|params| params.is_empty())
                {
                    return Err("Elite ES66 status does not accept arguments".into());
                }
                Operation::Status
            }
            motion::ACTION => Operation::MoveXyz(
                serde_json::from_value(cmd.params.clone())
                    .map_err(|_| "Elite ES66 move_xyz requires numeric x_m, y_m and z_m")?,
            ),
            _ => return Err("unsupported Elite ES66 action".into()),
        };
        serde_json::to_vec(&Request {
            command_id: cmd.command_id.clone(),
            device_id: cmd.device_id.clone(),
            operation,
        })
        .map_err(|_| "could not encode Elite ES66 request".into())
    }

    fn decode_response(&self, _device_type: &str, _bytes: &[u8]) -> Option<HashMap<String, Value>> {
        None
    }
}

struct Robot {
    id: String,
    transport_id: String,
    host: Ipv4Addr,
    motion_task: Option<String>,
    motion_active: Mutex<()>,
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
            }]
            .into_iter()
            .chain(self.motion_task.as_deref().map(motion::schema))
            .collect(),
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
        "Elite ES66 controller".into()
    }

    async fn send(&self, _bytes: &[u8]) -> Result<(), String> {
        Err("Elite ES66 accepts only typed requests".into())
    }

    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        let request: Request =
            serde_json::from_slice(bytes).map_err(|_| "invalid Elite ES66 request")?;
        if request.device_id != self.id {
            return Err("Elite ES66 request targets another device".into());
        }
        let (message, data) = match request.operation {
            Operation::Status => {
                let sample = self.sample().await?;
                let data = observed_command_data(
                    &request.device_id,
                    sample.observed_at_ms,
                    sample.observation(),
                    &sample,
                );
                ("Elite ES66 status read completed", data)
            }
            Operation::MoveXyz(target) => {
                let task = self
                    .motion_task
                    .as_deref()
                    .ok_or("Elite ES66 XYZ control is disabled")?;
                let _active = self
                    .motion_active
                    .try_lock()
                    .map_err(|_| "Elite ES66 already has an active XYZ request")?;
                if *self.stopped.lock().await {
                    return Err("Elite ES66 transport stopped".into());
                }
                let result = motion::execute(self.host, task, target).await?;
                let data = observed_command_data(
                    &request.device_id,
                    result.observed.observed_at_ms,
                    result.observed.observation(),
                    &result,
                );
                ("Elite ES66 XYZ target reached", data)
            }
        };
        Ok(Some(CommandResult {
            command_id: request.command_id,
            device_id: request.device_id,
            status: CommandStatus::Succeeded,
            message: message.into(),
            data: Some(data),
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
            if config
                .motion_task
                .as_deref()
                .is_some_and(|name| !motion::valid_task_name(name))
            {
                return Err("Elite ES66 motion_task must be a .task filename".into());
            }
            robots.push(Arc::new(Robot {
                id: config.id.clone(),
                transport_id: format!("{ADAPTER}:{}", config.id),
                host: config.host,
                motion_task: config.motion_task.clone(),
                motion_active: Mutex::default(),
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
