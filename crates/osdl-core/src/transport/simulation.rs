//! Device handles into a single deterministic simulation world.
use super::Transport;
use super::TransportRx;
use crate::config::{SimulationConfig, SimulationDeviceConfig};
use crate::protocol::{CommandResult, CommandStatus, DeviceCommand};
use crate::simulation::{
    basic::KinematicBackend, world::WorldRuntime, InstrumentState, SampleInventory,
};
use async_trait::async_trait;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct SimulationState {
    pub initial_properties: HashMap<String, Value>,
    pub properties: HashMap<String, Value>,
    pub instrument: Option<InstrumentState>,
    pub step: u64,
    pub revision: u64,
    pub last_action: Option<String>,
}

pub trait SimulationBackend: Send + Sync {
    fn engine_id(&self) -> &str;
    fn apply_action(
        &self,
        state: &mut SimulationState,
        samples: &mut SampleInventory,
        device: &str,
        action: &str,
        params: &Value,
    ) -> Result<(), String>;
    fn advance(&self, state: &mut SimulationState, dt: f64) -> bool;
    fn reset(&self, state: &mut SimulationState) {
        state.properties = state.initial_properties.clone();
        state.step = 0;
        state.last_action = Some("reset".into());
    }
}

pub trait SimulationBackendFactory: Send + Sync {
    fn create(
        &self,
        engine: &str,
        world_id: &str,
        device: &SimulationDeviceConfig,
    ) -> Result<Arc<dyn SimulationBackend>, String>;
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
        match engine {
            "kinematic" => Ok(Arc::new(KinematicBackend)),
            _ => Err(format!(
                "simulation engine '{engine}' is unavailable; install its runtime adapter"
            )),
        }
    }
}

#[derive(Clone)]
pub struct SimulationTransport {
    world: Arc<WorldRuntime>,
    device: String,
}
impl SimulationTransport {
    pub fn for_device(world: Arc<WorldRuntime>, device: String) -> Self {
        Self { world, device }
    }
    pub fn new(
        world_id: String,
        engine: String,
        device: &SimulationDeviceConfig,
        tick_hz: u32,
        rx_tx: mpsc::UnboundedSender<TransportRx>,
    ) -> Result<Self, String> {
        let config = SimulationConfig {
            world_id,
            engine,
            tick_hz,
            devices: vec![device.clone()],
            samples: Vec::new(),
            seed: 0,
        };
        let world = WorldRuntime::new(&config, rx_tx, &BuiltinSimulationBackendFactory)?;
        Ok(Self::for_device(world, device.id.clone()))
    }
    async fn dispatch(&self, bytes: &[u8]) -> Result<CommandResult, String> {
        if !self.world.is_running() {
            return Err("simulation world is not running".into());
        }
        let cmd: DeviceCommand =
            serde_json::from_slice(bytes).map_err(|e| format!("simulation command: {e}"))?;
        if cmd.device_id != self.world.device_id(&self.device) {
            return Err("command targets a different device".into());
        }
        let result = self
            .world
            .apply(&self.device, &cmd.action, &cmd.params)
            .await;
        Ok(CommandResult {
            command_id: cmd.command_id,
            device_id: cmd.device_id,
            status: if result.is_ok() {
                CommandStatus::Succeeded
            } else {
                CommandStatus::Failed
            },
            message: result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| "completed".into()),
            data: result.ok(),
        })
    }
}
#[async_trait]
impl Transport for SimulationTransport {
    fn transport_type(&self) -> &str {
        "simulation"
    }
    fn description(&self) -> String {
        format!("Simulation {}", self.device)
    }
    async fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let result = self.dispatch(bytes).await?;
        if result.status == CommandStatus::Failed {
            Err(result.message)
        } else {
            Ok(())
        }
    }
    async fn execute(&self, bytes: &[u8]) -> Result<Option<CommandResult>, String> {
        self.dispatch(bytes).await.map(Some)
    }
    fn is_connected(&self) -> bool {
        self.world.is_running()
    }
    async fn start(&self) -> Result<(), String> {
        self.world.start().await;
        Ok(())
    }
    async fn stop(&self) -> Result<(), String> {
        self.world.stop().await;
        Ok(())
    }
}
pub fn transport_id_for(world_id: &str, device_id: &str) -> String {
    format!("simulation:{world_id}:{device_id}")
}
pub fn device_id_for(world_id: &str, device_id: &str) -> String {
    format!("sim:{world_id}:{device_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SimulationConfig;
    use crate::protocol::DeviceCommand;
    use serde_json::json;

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

    #[tokio::test]
    async fn reset_restores_configured_properties_and_step() {
        let config = SimulationConfig::default();
        let device = config.devices.first().expect("default heater");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let transport =
            SimulationTransport::new(config.world_id, config.engine, device, config.tick_hz, tx)
                .expect("kinematic backend");
        transport.start().await.expect("start");
        let _ = rx.recv().await.expect("initial telemetry");

        let set_temperature = DeviceCommand {
            command_id: "reset-temperature-command".into(),
            device_id: device_id_for("lab-sim", &device.id),
            action: "set_temperature".into(),
            params: json!({"temperature": 80.0}),
        };
        transport
            .send(&serde_json::to_vec(&set_temperature).expect("encode"))
            .await
            .expect("set temperature");
        let _ = rx.recv().await.expect("temperature telemetry");

        let reset = DeviceCommand {
            command_id: "reset-command".into(),
            device_id: device_id_for("lab-sim", &device.id),
            action: "reset".into(),
            params: json!({}),
        };
        transport
            .send(&serde_json::to_vec(&reset).expect("encode"))
            .await
            .expect("reset");
        let telemetry = rx.recv().await.expect("reset telemetry");
        let payload: Value = serde_json::from_slice(&telemetry.data).expect("json");
        assert_eq!(payload["properties"]["temperature"], json!(22.0));
        assert_eq!(payload["properties"]["target_temperature"], json!(22.0));
        assert_eq!(payload["properties"]["step"], json!(0));
        assert_eq!(payload["last_action"], json!("reset"));
        transport.stop().await.expect("stop");
    }
}
