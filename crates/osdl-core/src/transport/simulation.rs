//! Deterministic local simulation transport.
//!
//! The built-in backend is intentionally small: it provides stable device
//! state and telemetry for developing Lab Action Model workflows without a
//! connected instrument. A future Rapier, Bullet, MuJoCo, Isaac, or Gazebo
//! integration can implement the same contract behind a new backend while
//! retaining the engine's device/action/event surface.

use super::{Transport, TransportRx};
use crate::config::SimulationDeviceConfig;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct SimulationTransport {
    inner: Arc<Inner>,
}

struct Inner {
    transport_id: String,
    world_id: String,
    device_id: String,
    role: Option<String>,
    position: [f64; 3],
    tick_hz: u32,
    state: Mutex<SimulationState>,
    backend: Arc<dyn SimulationBackend>,
    rx_tx: mpsc::UnboundedSender<TransportRx>,
    tick_task: Mutex<Option<JoinHandle<()>>>,
    connected: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct SimulationState {
    pub properties: HashMap<String, Value>,
    pub step: u64,
    pub last_action: Option<String>,
}

/// Runtime seam for physics integrations. The transport owns identity,
/// telemetry, and the OpenSDL event path; a backend owns world evolution and
/// action semantics. A future Rapier/Bullet/MuJoCo/Isaac/Gazebo adapter can
/// implement this seam without changing the device contract.
#[async_trait]
pub trait SimulationBackend: Send + Sync {
    fn engine_id(&self) -> &str;
    async fn apply_action(&self, state: &mut SimulationState, action: &str, params: &Value);
    async fn advance(&self, state: &mut SimulationState, dt: f64);
}

/// Creates a physics backend for one configured simulation world.
///
/// Hosts that embed OpenSDL can inject a factory for a native runtime while
/// keeping device identity, commands, telemetry, and events on the OpenSDL
/// contract. The default implementation only exposes the deterministic
/// kinematic backend shipped in this crate.
pub trait SimulationBackendFactory: Send + Sync {
    fn create(
        &self,
        engine: &str,
        world_id: &str,
        device: &SimulationDeviceConfig,
    ) -> Result<Arc<dyn SimulationBackend>, String>;
}

struct KinematicBackend;

#[async_trait]
impl SimulationBackend for KinematicBackend {
    fn engine_id(&self) -> &str {
        "kinematic"
    }

    async fn apply_action(&self, state: &mut SimulationState, action: &str, params: &Value) {
        state.last_action = Some(action.to_string());
        match action {
            "start" | "resume" => {
                state.properties.insert("running".into(), Value::Bool(true));
            }
            "stop" | "pause" => {
                state
                    .properties
                    .insert("running".into(), Value::Bool(false));
            }
            "open" => {
                state
                    .properties
                    .insert("state".into(), Value::String("open".into()));
            }
            "close" => {
                state
                    .properties
                    .insert("state".into(), Value::String("closed".into()));
            }
            "set_temperature" | "set_target_temperature" => {
                if let Some(value) = number_param(params, &["temperature", "target", "value"]) {
                    state
                        .properties
                        .insert("target_temperature".into(), json!(value));
                }
            }
            "set_speed" | "set_stir_speed" => {
                if let Some(value) = number_param(params, &["speed", "rpm", "value"]) {
                    state
                        .properties
                        .insert("speed".into(), json!(value.max(0.0)));
                }
            }
            "set_position" | "move" => {
                if let Some(value) = number_param(params, &["position", "value"]) {
                    state
                        .properties
                        .insert("position_value".into(), json!(value));
                }
            }
            "set_asset" => {
                let namespace = params.get("namespace").and_then(Value::as_str);
                let name = params.get("name").and_then(Value::as_str);
                if let (Some(namespace), Some(name)) = (namespace, name) {
                    let mut asset = json!({
                        "namespace": namespace,
                        "name": name,
                    });
                    if let Some(version) = params.get("version").and_then(Value::as_str) {
                        asset["version"] = json!(version);
                    }
                    state.properties.insert("simulation_asset".into(), asset);
                }
            }
            "clear_asset" => {
                state.properties.remove("simulation_asset");
            }
            _ => {
                state
                    .properties
                    .insert("last_action_params".into(), params.clone());
            }
        }
    }

    async fn advance(&self, state: &mut SimulationState, dt: f64) {
        state.step = state.step.saturating_add(1);
        if let (Some(Value::Number(current)), Some(Value::Number(target))) = (
            state.properties.get("temperature"),
            state.properties.get("target_temperature"),
        ) {
            let current = current.as_f64().unwrap_or(22.0);
            let target = target.as_f64().unwrap_or(current);
            let running = state
                .properties
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let next = if running {
                current + (target - current) * (1.0 - (-0.8 * dt).exp())
            } else {
                current
            };
            state.properties.insert("temperature".into(), json!(next));
        }
        if let Some(speed) = state.properties.get("speed").and_then(Value::as_f64) {
            let running = state
                .properties
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.properties.insert(
                "angular_velocity".into(),
                json!(if running { speed } else { 0.0 }),
            );
        }
    }
}

fn backend_for(engine: &str) -> Result<Arc<dyn SimulationBackend>, String> {
    match engine {
        "kinematic" => Ok(Arc::new(KinematicBackend)),
        other => Err(format!(
            "simulation engine '{other}' is not available in this OpenSDL build; use 'kinematic' or install a runtime adapter"
        )),
    }
}

#[derive(Debug, Default)]
pub struct BuiltinSimulationBackendFactory;

impl SimulationBackendFactory for BuiltinSimulationBackendFactory {
    fn create(
        &self,
        engine: &str,
        _world_id: &str,
        _device: &SimulationDeviceConfig,
    ) -> Result<Arc<dyn SimulationBackend>, String> {
        backend_for(engine)
    }
}

impl SimulationTransport {
    pub fn new(
        world_id: String,
        engine: String,
        device: &SimulationDeviceConfig,
        tick_hz: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Result<Self, String> {
        let backend =
            BuiltinSimulationBackendFactory::default().create(&engine, &world_id, device)?;
        Self::with_backend(world_id, device, tick_hz, rx_tx, backend)
    }

    /// Construct a transport from an externally provided physics backend.
    /// Desktop and headless hosts can register a Rapier, Bullet, MuJoCo,
    /// Isaac, or Gazebo implementation here without changing OpenSDL's
    /// device/action/event contract.
    pub fn with_backend(
        world_id: String,
        device: &SimulationDeviceConfig,
        tick_hz: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
        backend: Arc<dyn SimulationBackend>,
    ) -> Result<Self, String> {
        if tick_hz == 0 || tick_hz > 240 {
            return Err("simulation tick_hz must be between 1 and 240".into());
        }
        let transport_id = transport_id_for(&world_id, &device.id);
        let engine = backend.engine_id().to_string();
        let mut properties = device.properties.clone();
        properties.insert("simulation".into(), Value::Bool(true));
        properties.insert("simulation_engine".into(), Value::String(engine.clone()));
        properties.insert("simulation_world".into(), Value::String(world_id.clone()));
        properties.insert("position".into(), json!(device.position));
        if let Some(asset_ref) = &device.asset_ref {
            properties.insert(
                "simulation_asset".into(),
                json!({
                    "namespace": asset_ref.namespace,
                    "name": asset_ref.name,
                    "version": asset_ref.version,
                }),
            );
        }

        Ok(Self {
            inner: Arc::new(Inner {
                transport_id,
                world_id: world_id.clone(),
                device_id: device_id_for(&world_id, &device.id),
                role: device.role.clone(),
                position: device.position,
                tick_hz,
                state: Mutex::new(SimulationState {
                    properties,
                    step: 0,
                    last_action: None,
                }),
                backend,
                rx_tx,
                tick_task: Mutex::new(None),
                connected: AtomicBool::new(false),
            }),
        })
    }

    async fn emit_status(&self) {
        let state = self.inner.state.lock().await.clone();
        let mut properties = state.properties;
        properties.insert("step".into(), json!(state.step));
        let envelope = json!({
            "device_id": self.inner.device_id,
            "world_id": self.inner.world_id,
            "engine": self.inner.backend.engine_id(),
            "role": self.inner.role,
            "position": self.inner.position,
            "timestamp": now_millis(),
            "last_action": state.last_action,
            "properties": properties,
        });
        let _ = self.inner.rx_tx.send(TransportRx {
            transport_id: self.inner.transport_id.clone(),
            data: serde_json::to_vec(&envelope).unwrap_or_default(),
        });
    }

    async fn apply_action(&self, action: &str, params: &Value) {
        let mut state = self.inner.state.lock().await;
        self.inner
            .backend
            .apply_action(&mut state, action, params)
            .await;
    }

    async fn advance(&self) {
        let dt = 1.0 / self.inner.tick_hz as f64;
        let mut state = self.inner.state.lock().await;
        self.inner.backend.advance(&mut state, dt).await;
    }
}

#[async_trait]
impl Transport for SimulationTransport {
    fn transport_type(&self) -> &str {
        "simulation"
    }

    fn description(&self) -> String {
        format!(
            "Simulation {} ({})",
            self.inner.device_id,
            self.inner.backend.engine_id()
        )
    }

    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        if !self.is_connected() {
            return Err("simulation transport is not running".into());
        }
        let command: Value =
            serde_json::from_slice(bytes).map_err(|e| format!("simulation: parse command: {e}"))?;
        let action = command
            .get("action")
            .and_then(Value::as_str)
            .ok_or("simulation: command missing action")?;
        let params = command.get("params").cloned().unwrap_or_else(|| json!({}));
        if action == "set_asset" {
            let namespace = params
                .get("namespace")
                .and_then(Value::as_str)
                .unwrap_or("");
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if namespace.trim().is_empty() || name.trim().is_empty() {
                return Err("simulation: set_asset requires namespace and name".into());
            }
            if params
                .get("version")
                .and_then(Value::as_str)
                .is_some_and(|version| version.trim().is_empty())
            {
                return Err("simulation: set_asset version must not be empty".into());
            }
        }
        self.apply_action(action, &params).await;
        self.emit_status().await;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Acquire)
    }

    async fn start(&self) -> Result<(), String> {
        if self.inner.connected.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.emit_status().await;
        let this = self.clone();
        let interval = std::time::Duration::from_secs_f64(1.0 / this.inner.tick_hz as f64);
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if !this.is_connected() {
                    break;
                }
                this.advance().await;
                this.emit_status().await;
            }
        });
        *self.inner.tick_task.lock().await = Some(task);
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        self.inner.connected.store(false, Ordering::Release);
        if let Some(task) = self.inner.tick_task.lock().await.take() {
            task.abort();
        }
        Ok(())
    }
}

pub fn transport_id_for(world_id: &str, device_id: &str) -> String {
    format!("simulation:{world_id}:{device_id}")
}

pub fn device_id_for(world_id: &str, device_id: &str) -> String {
    format!("sim:{world_id}:{device_id}")
}

fn number_param(params: &Value, names: &[&str]) -> Option<f64> {
    names
        .iter()
        .find_map(|name| params.get(*name).and_then(Value::as_f64))
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SimulationConfig;
    use crate::protocol::DeviceCommand;

    #[tokio::test]
    async fn emits_telemetry_and_applies_actions() {
        let config = SimulationConfig::default();
        let device = config.devices.first().expect("default heater");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let transport =
            SimulationTransport::new(config.world_id, config.engine, device, config.tick_hz, tx)
                .expect("kinematic backend");
        transport.start().await.expect("start");
        let initial = rx.recv().await.expect("initial telemetry");
        assert_eq!(
            initial.transport_id,
            transport_id_for("lab-sim", "heater-1")
        );

        let command = DeviceCommand {
            command_id: "command-1".into(),
            device_id: device_id_for("lab-sim", &device.id),
            action: "set_temperature".into(),
            params: json!({"temperature": 80.0}),
        };
        transport
            .send(&serde_json::to_vec(&command).expect("encode"))
            .await
            .expect("send");
        let update = rx.recv().await.expect("action telemetry");
        let payload: Value = serde_json::from_slice(&update.data).expect("json");
        assert_eq!(payload["properties"]["target_temperature"], json!(80.0));
        transport.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn binds_and_clears_a_hub_asset_at_runtime() {
        let config = SimulationConfig::default();
        let device = config.devices.first().expect("default heater");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let transport =
            SimulationTransport::new(config.world_id, config.engine, device, config.tick_hz, tx)
                .expect("kinematic backend");
        transport.start().await.expect("start");
        let _ = rx.recv().await.expect("initial telemetry");

        let command = DeviceCommand {
            command_id: "asset-command".into(),
            device_id: device_id_for("lab-sim", &device.id),
            action: "set_asset".into(),
            params: json!({
                "namespace": "scienceol",
                "name": "heater-dalong",
                "version": "1.0.0"
            }),
        };
        transport
            .send(&serde_json::to_vec(&command).expect("encode"))
            .await
            .expect("bind asset");
        let bound = rx.recv().await.expect("asset telemetry");
        let payload: Value = serde_json::from_slice(&bound.data).expect("json");
        assert_eq!(
            payload["properties"]["simulation_asset"],
            json!({
                "namespace": "scienceol",
                "name": "heater-dalong",
                "version": "1.0.0"
            })
        );

        let clear = DeviceCommand {
            command_id: "clear-asset-command".into(),
            device_id: device_id_for("lab-sim", &device.id),
            action: "clear_asset".into(),
            params: json!({}),
        };
        transport
            .send(&serde_json::to_vec(&clear).expect("encode"))
            .await
            .expect("clear asset");
        let cleared = rx.recv().await.expect("cleared telemetry");
        let payload: Value = serde_json::from_slice(&cleared.data).expect("json");
        assert!(payload["properties"].get("simulation_asset").is_none());
        transport.stop().await.expect("stop");
    }
}
