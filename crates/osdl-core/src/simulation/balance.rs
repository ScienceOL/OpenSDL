use super::SampleInventory;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BalanceConfig {
    pub model_id: String,
    pub capacity_kg: f64,
    pub readability_kg: f64,
    pub settling_s: f64,
    pub door_travel_m: f64,
    pub pan_radius_m: f64,
}

impl BalanceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || [
                self.capacity_kg,
                self.readability_kg,
                self.settling_s,
                self.door_travel_m,
                self.pan_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.readability_kg > self.capacity_kg
        {
            return Err("invalid analytical balance parameters".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BalanceState {
    pub config: BalanceConfig,
    powered: bool,
    doors: [f64; 3],
    gross_kg: f64,
    tare_kg: f64,
    elapsed_s: f64,
    stable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DoorParams {
    door: Door,
    opening_m: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Door {
    Left,
    Right,
    Top,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleParams {
    sample_id: String,
}

impl BalanceState {
    pub fn new(config: BalanceConfig) -> Self {
        Self {
            config,
            powered: false,
            doors: [0.0; 3],
            gross_kg: 0.0,
            tare_kg: 0.0,
            elapsed_s: 0.0,
            stable: false,
        }
    }

    fn disturbed(&mut self) {
        self.elapsed_s = 0.0;
        self.stable = false;
    }

    pub fn apply(
        &mut self,
        device: &str,
        action: &str,
        params: &Value,
        samples: &mut SampleInventory,
    ) -> Result<(), String> {
        match action {
            "power" => {
                empty(params)?;
                self.powered = !self.powered;
                self.disturbed();
            }
            "set_door" => {
                let p: DoorParams =
                    serde_json::from_value(params.clone()).map_err(|_| "INVALID_PARAMS")?;
                if !p.opening_m.is_finite()
                    || !(0.0..=self.config.door_travel_m).contains(&p.opening_m)
                {
                    return Err("DOOR_LIMIT".into());
                }
                let index = match p.door {
                    Door::Left => 0,
                    Door::Right => 1,
                    Door::Top => 2,
                };
                if self.doors[index] != p.opening_m {
                    self.doors[index] = p.opening_m;
                    self.disturbed();
                }
            }
            "load_sample" | "unload_sample" => {
                let p: SampleParams =
                    serde_json::from_value(params.clone()).map_err(|_| "INVALID_PARAMS")?;
                if self
                    .doors
                    .iter()
                    .all(|v| *v < self.config.door_travel_m * 0.7)
                {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                let delta = if action == "load_sample" {
                    samples.claim(&p.sample_id, device, self.config.pan_radius_m)?
                } else {
                    -samples.release(&p.sample_id, device)?
                };
                self.gross_kg = (self.gross_kg + delta).max(0.0);
                self.disturbed();
            }
            "tare" | "zero" => {
                empty(params)?;
                if !self.powered || !self.stable {
                    return Err("WAIT_FOR_STABLE_LOAD".into());
                }
                if action == "zero" && self.gross_kg > self.config.readability_kg {
                    return Err("UNLOAD_BEFORE_ZERO".into());
                }
                self.tare_kg = if action == "tare" { self.gross_kg } else { 0.0 };
            }
            "reset" => {
                empty(params)?;
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }

    pub fn advance(&mut self, dt: f64) -> bool {
        if self.powered
            && self.doors.iter().all(|v| *v <= 1e-9)
            && self.gross_kg <= self.config.capacity_kg
            && !self.stable
        {
            self.elapsed_s += dt;
            if self.elapsed_s + 1e-9 >= self.config.settling_s {
                self.stable = true;
                return true;
            }
        }
        false
    }

    pub fn properties(
        &self,
        device: &str,
        run_id: &str,
        samples: &SampleInventory,
        revision: u64,
        time_s: f64,
    ) -> HashMap<String, Value> {
        let overload = self.gross_kg > self.config.capacity_kg;
        let mass = (self.gross_kg - self.tare_kg) / self.config.readability_kg;
        let mass_g =
            (self.powered && !overload).then(|| mass.round() * self.config.readability_kg * 1000.0);
        let status = if !self.powered {
            "off"
        } else if overload {
            "overload"
        } else if self.stable {
            "stable"
        } else {
            "unstable"
        };
        let mut values = json!({"mass_g": mass_g, "powered": self.powered, "stable": self.stable,
            "status": status, "door_left_m": self.doors[0], "door_right_m": self.doors[1], "door_top_m": self.doors[2]});
        for (id, visible) in samples.visibility(device) {
            values[format!("{id}_visible")] = json!(visible);
        }
        HashMap::from([
            (
                "model_state".into(),
                json!({"schema_version":2, "model_id":self.config.model_id, "run_id":run_id,
            "revision":revision,"time_s":time_s,"values":values,"motion":{}}),
            ),
            ("mass_g".into(), json!(mass_g)),
            ("balance_status".into(), json!(status)),
            ("running".into(), json!(self.powered)),
        ])
    }
}

fn empty(value: &Value) -> Result<(), String> {
    if value.as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err("INVALID_PARAMS".into())
    }
}
