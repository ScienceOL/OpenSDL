use super::{InstrumentState, SampleInventory};
use crate::{
    config::{SimulationConfig, SimulationDeviceConfig},
    transport::{
        simulation::{
            device_id_for, transport_id_for, SimulationBackend, SimulationBackendFactory,
            SimulationState,
        },
        TransportRx,
    },
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

struct DeviceRuntime {
    config: SimulationDeviceConfig,
    backend: Arc<dyn SimulationBackend>,
    state: SimulationState,
}
impl DeviceRuntime {
    fn new(
        config: &SimulationConfig,
        c: &SimulationDeviceConfig,
        factory: &dyn SimulationBackendFactory,
    ) -> Result<Self, String> {
        let instrument = c.model.as_ref().map(InstrumentState::new);
        let mut initial_properties = c.properties.clone();
        if let Some(model) = &c.model {
            initial_properties.insert("model_conditions".into(), json!(model.condition_channels()));
        }
        if let Some(asset) = &c.asset_ref {
            initial_properties.insert("simulation_asset".into(), json!(asset));
        }
        Ok(DeviceRuntime {
            config: c.clone(),
            backend: factory.create(&config.engine, &config.world_id, c)?,
            state: SimulationState {
                initial_properties: initial_properties.clone(),
                properties: initial_properties,
                instrument,
                step: 0,
                revision: 0,
                last_action: None,
            },
        })
    }
}
struct WorldState {
    devices: HashMap<String, DeviceRuntime>,
    samples: SampleInventory,
    time_s: f64,
}

/// One state owner and clock for every device and sample in a simulation instance.
pub struct WorldRuntime {
    world_id: String,
    run_id: String,
    tick_hz: AtomicU32,
    scene_revision: AtomicU64,
    state: Mutex<WorldState>,
    tx: mpsc::UnboundedSender<TransportRx>,
    running: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
}

pub struct DevicePose {
    pub id: String,
    pub position: [f64; 3],
    pub rotation: [f64; 3],
    pub parent: Option<String>,
}

impl WorldRuntime {
    pub fn new(
        config: &SimulationConfig,
        tx: mpsc::UnboundedSender<TransportRx>,
        factory: &dyn SimulationBackendFactory,
    ) -> Result<Arc<Self>, String> {
        config.validate()?;
        let mut devices = HashMap::new();
        for c in &config.devices {
            devices.insert(c.id.clone(), DeviceRuntime::new(config, c, factory)?);
        }
        Ok(Arc::new(Self {
            world_id: config.world_id.clone(),
            run_id: uuid::Uuid::new_v4().simple().to_string(),
            tick_hz: AtomicU32::new(config.tick_hz),
            scene_revision: AtomicU64::new(config.scene_revision),
            state: Mutex::new(WorldState {
                devices,
                samples: SampleInventory::new(&config.samples)?,
                time_s: 0.0,
            }),
            tx,
            running: AtomicBool::new(false),
            task: Mutex::new(None),
        }))
    }

    pub(crate) fn scene_revision(&self) -> u64 {
        self.scene_revision.load(Ordering::Relaxed)
    }

    pub async fn apply_layout(
        &self,
        revision: u64,
        poses: &[DevicePose],
        added: &HashSet<String>,
    ) -> Vec<crate::protocol::DeviceStatus> {
        let mut statuses = Vec::new();
        let mut world = self.state.lock().await;
        for DevicePose {
            id,
            position,
            rotation,
            parent,
        } in poses
        {
            let rotation = json!(rotation);
            let parent = json!(parent);
            let Some(device) = world.devices.get_mut(id) else {
                continue;
            };
            if !added.contains(id)
                && device.config.position == *position
                && device.state.properties.get("rotation") == Some(&rotation)
                && device.state.properties.get("parent") == Some(&parent)
            {
                continue;
            }
            device.config.position = *position;
            for props in [
                &mut device.state.properties,
                &mut device.state.initial_properties,
            ] {
                props.insert("scene_revision".into(), json!(revision));
                props.insert("rotation".into(), rotation.clone());
                props.insert("parent".into(), parent.clone());
            }
            let device = &world.devices[id];
            statuses.push(crate::protocol::DeviceStatus {
                device_id: self.device_id(id),
                timestamp: crate::engine::now_millis(),
                properties: self.observations(device, &world),
            });
        }
        self.scene_revision.store(revision, Ordering::Relaxed);
        statuses
    }
    pub async fn reconcile(
        &self,
        config: &SimulationConfig,
        factory: &dyn SimulationBackendFactory,
    ) -> Result<(Vec<String>, HashSet<String>), String> {
        let mut world = self.state.lock().await;
        let mut additions = Vec::new();
        let mut samples = SampleInventory::new(&config.samples)?;
        let retained: HashSet<_> = config
            .devices
            .iter()
            .filter_map(|c| {
                world
                    .devices
                    .get(&c.id)
                    .filter(|d| {
                        d.config.properties.get("model_manifest_digest")
                            == c.properties.get("model_manifest_digest")
                    })
                    .map(|_| c.id.clone())
            })
            .collect();
        for c in &config.devices {
            if !retained.contains(&c.id) {
                additions.push((c.id.clone(), DeviceRuntime::new(config, c, factory)?));
            }
        }
        let removed = world
            .devices
            .keys()
            .filter(|id| !retained.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        samples.retain_owners(&world.samples, &retained);
        world.samples = samples;
        self.tick_hz.store(config.tick_hz, Ordering::Relaxed);
        for id in &removed {
            world.devices.remove(id);
        }
        let added = additions.iter().map(|(id, _)| id.clone()).collect();
        world.devices.extend(additions);
        Ok((removed, added))
    }
    pub fn device_id(&self, id: &str) -> String {
        device_id_for(&self.world_id, id)
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub async fn start(self: &Arc<Self>) {
        let mut task = self.task.lock().await;
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        {
            let state = self.state.lock().await;
            for device in state.devices.values() {
                self.emit(device, &state);
            }
        }
        let world = self.clone();
        *task = Some(tokio::spawn(async move {
            let mut deadline = tokio::time::Instant::now();
            while world.is_running() {
                let dt = 1.0 / f64::from(world.tick_hz.load(Ordering::Relaxed));
                deadline += Duration::from_secs_f64(dt);
                tokio::time::sleep_until(deadline).await;
                world.advance(dt).await;
                deadline = deadline.max(tokio::time::Instant::now());
            }
        }));
    }
    pub async fn stop(&self) {
        let mut task = self.task.lock().await;
        self.running.store(false, Ordering::Release);
        if let Some(task) = task.take() {
            task.abort();
        }
    }
    pub async fn apply(&self, id: &str, action: &str, params: &Value) -> Result<Value, String> {
        let mut world = self.state.lock().await;
        let WorldState {
            devices, samples, ..
        } = &mut *world;
        let device = devices.get_mut(id).ok_or("UNKNOWN_DEVICE")?;
        if device.config.device_type == "simulation.static" {
            return Err("NO_DEVICE_ACTIONS".into());
        }
        device
            .backend
            .apply_action(&mut device.state, samples, id, action, params)?;
        device.state.last_action = Some(action.to_owned());
        device.state.revision = device.state.revision.saturating_add(1);
        let device = world.devices.get(id).expect("validated device");
        let snapshot = self.snapshot(device, &world);
        self.send(device, &snapshot);
        Ok(snapshot)
    }
    pub async fn advance(&self, dt: f64) {
        let mut world = self.state.lock().await;
        world.time_s += dt;
        let mut changed = Vec::new();
        for (id, device) in &mut world.devices {
            if device.backend.advance(&mut device.state, dt) {
                device.state.step = device.state.step.saturating_add(1);
                device.state.revision = device.state.revision.saturating_add(1);
                changed.push(id.clone());
            }
        }
        for id in changed {
            self.emit(&world.devices[&id], &world);
        }
    }
    fn observations(&self, device: &DeviceRuntime, world: &WorldState) -> HashMap<String, Value> {
        let state = &device.state;
        let config = &device.config;
        let mut properties = state.properties.clone();
        if let Some(instrument) = &state.instrument {
            properties.extend(instrument.properties(
                &config.id,
                &self.run_id,
                &world.samples,
                state.revision,
                world.time_s,
            ));
        }
        properties.insert("position".into(), json!(config.position));
        let mut transitions = serde_json::Map::new();
        if let Some(values) = properties
            .get("model_state")
            .and_then(|model| model["values"].as_object())
        {
            for channel in values.keys() {
                transitions.insert(
                    channel.clone(),
                    json!(device.backend.transition(channel, state)),
                );
            }
        }
        properties.insert("transition".into(), Value::Object(transitions));
        properties.insert("step".into(), json!(state.step));
        properties.insert("simulation".into(), json!(true));
        properties.insert("simulation_world".into(), json!(self.world_id));
        properties.insert(
            "simulation_engine".into(),
            json!(device.backend.engine_id()),
        );
        properties.insert("last_action".into(), json!(state.last_action));
        properties
    }
    fn snapshot(&self, device: &DeviceRuntime, world: &WorldState) -> Value {
        let config = &device.config;
        let state = &device.state;
        let properties = self.observations(device, world);
        json!({"device_id":self.device_id(&config.id),"world_id":self.world_id,
            "engine":device.backend.engine_id(),"role":config.role,"position":config.position,
            "timestamp":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
            "last_action":state.last_action,"properties":properties})
    }
    fn emit(&self, device: &DeviceRuntime, world: &WorldState) {
        self.send(device, &self.snapshot(device, world));
    }
    fn send(&self, device: &DeviceRuntime, envelope: &Value) {
        if let Ok(data) = serde_json::to_vec(envelope) {
            let _ = self.tx.send(TransportRx {
                transport_id: transport_id_for(&self.world_id, &device.config.id),
                data,
            });
        }
    }
}
