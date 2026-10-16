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
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
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
struct WorldState {
    devices: HashMap<String, DeviceRuntime>,
    samples: SampleInventory,
    time_s: f64,
}

/// One state owner and clock for every device and sample in a simulation instance.
pub struct WorldRuntime {
    world_id: String,
    run_id: String,
    tick_hz: u32,
    state: Mutex<WorldState>,
    tx: mpsc::UnboundedSender<TransportRx>,
    running: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
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
            let instrument = c.model.as_ref().map(InstrumentState::new);
            let mut initial_properties = c.properties.clone();
            if let Some(asset) = &c.asset_ref {
                initial_properties.insert("simulation_asset".into(), json!(asset));
            }
            devices.insert(
                c.id.clone(),
                DeviceRuntime {
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
                },
            );
        }
        Ok(Arc::new(Self {
            world_id: config.world_id.clone(),
            run_id: uuid::Uuid::new_v4().simple().to_string(),
            tick_hz: config.tick_hz,
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
            let period = Duration::from_secs_f64(1.0 / world.tick_hz as f64);
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while world.is_running() {
                ticker.tick().await;
                world.advance(1.0 / world.tick_hz as f64).await;
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
    fn snapshot(&self, device: &DeviceRuntime, world: &WorldState) -> Value {
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
        properties.insert("step".into(), json!(state.step));
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
