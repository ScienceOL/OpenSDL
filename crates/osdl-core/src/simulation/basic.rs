use crate::simulation::SampleInventory;
use crate::transport::simulation::{SimulationBackend, SimulationState};
use serde_json::{json, Value};

pub struct KinematicBackend;

impl SimulationBackend for KinematicBackend {
    fn engine_id(&self) -> &str {
        "kinematic"
    }

    fn apply_action(
        &self,
        state: &mut SimulationState,
        samples: &mut SampleInventory,
        device: &str,
        action: &str,
        params: &Value,
    ) -> Result<(), String> {
        if let Some(balance) = &mut state.balance {
            return balance.apply(device, action, params, samples);
        }
        match action {
            "reset" => self.reset(state),
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
                if namespace.is_none_or(|s| s.trim().is_empty())
                    || name.is_none_or(|s| s.trim().is_empty())
                    || params
                        .get("version")
                        .is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
                {
                    return Err("INVALID_ASSET_REF".into());
                }
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
        Ok(())
    }

    fn advance(&self, state: &mut SimulationState, dt: f64) -> bool {
        if let Some(balance) = &mut state.balance {
            return balance.advance(dt);
        }
        let previous = state.properties.clone();
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
        previous != state.properties
    }
}

fn number_param(params: &Value, names: &[&str]) -> Option<f64> {
    names
        .iter()
        .find_map(|name| params.get(*name).and_then(Value::as_f64))
}
