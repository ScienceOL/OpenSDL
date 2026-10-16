//! Original four-position rotor. Kinematics and static mass balance, no separation chemistry.
use super::{thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CentrifugeConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub rotor_radius_m: f64,
    pub lid_open_rad: f64,
    pub imbalance_kg: f64,
}
impl CentrifugeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || [
                self.max_rpm,
                self.slew_rpm_s,
                self.rotor_radius_m,
                self.lid_open_rad,
                self.imbalance_kg,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("invalid centrifuge parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct CentrifugeState {
    pub config: CentrifugeConfig,
    opened: bool,
    slots: [Option<f64>; 4],
    rpm: f64,
    target: f64,
    angle: f64,
    running: bool,
}
impl CentrifugeState {
    pub fn new(config: CentrifugeConfig) -> Self {
        Self {
            config,
            opened: false,
            slots: [None; 4],
            rpm: 0.0,
            target: 0.0,
            angle: 0.0,
            running: false,
        }
    }
    pub fn apply(
        &mut self,
        device: &str,
        action: &str,
        params: &Value,
        samples: &mut SampleInventory,
    ) -> Result<(), String> {
        match action {
            "set_speed" => self.target = thermal::number(params, "rpm", 0.0, self.config.max_rpm)?,
            "lid" => {
                thermal::empty(params)?;
                if self.rpm > 0.0 || self.running {
                    return Err("ROTOR_MOVING".into());
                }
                self.opened = !self.opened;
            }
            "load_tube" | "unload_tube" => {
                let slot = thermal::number(params, "slot", 0.0, 3.0)?;
                if slot.fract() != 0.0 {
                    return Err("INVALID_PARAMS".into());
                }
                let i = slot as usize;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                let id = format!("tube_{i}");
                if action == "load_tube" {
                    self.slots[i] =
                        Some(samples.claim(&id, device, self.config.rotor_radius_m + 0.010)?);
                } else {
                    samples.release(&id, device)?;
                    self.slots[i] = None;
                }
            }
            "start" => {
                thermal::empty(params)?;
                if self.target <= 0.0 {
                    return Err("SET_SPEED_FIRST".into());
                }
                if self.opened {
                    return Err("CLOSE_LID_FIRST".into());
                }
                // Vector sum in the authored four orthogonal rotor slots.
                if (self.slots[0].unwrap_or(0.0) - self.slots[2].unwrap_or(0.0))
                    .hypot(self.slots[1].unwrap_or(0.0) - self.slots[3].unwrap_or(0.0))
                    > self.config.imbalance_kg
                {
                    return Err("ROTOR_IMBALANCE".into());
                }
                if !self.slots.iter().any(Option::is_some) {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.running = true;
            }
            "stop" => {
                thermal::empty(params)?;
                self.running = false;
            }
            "reset" => {
                thermal::empty(params)?;
                if self.rpm > 0.0 {
                    return Err("ROTOR_MOVING".into());
                }
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let previous = (self.rpm, self.angle);
        self.rpm = thermal::approach(
            self.rpm,
            if self.running { self.target } else { 0.0 },
            self.config.slew_rpm_s,
            dt,
        );
        self.angle = (self.angle + self.rpm * std::f64::consts::TAU / 60.0 * dt)
            .rem_euclid(std::f64::consts::TAU);
        previous != (self.rpm, self.angle)
    }
    pub fn values(&self) -> Value {
        json!({
            "rpm":self.rpm,
            "target_rpm":self.target,
            "speed_dial_rad":self.target/self.config.max_rpm*std::f64::consts::TAU*0.75,
            "rotor_rad":self.angle,
            "rotor_rad_s":self.rpm*std::f64::consts::TAU/60.0,
            "lid_rad":if self.opened {self.config.lid_open_rad}else{0.0},
            "rcf_g":self.rpm.powi(2)*(std::f64::consts::TAU/60.0).powi(2)*self.config.rotor_radius_m/9.80665,
            "tube_0_visible":self.slots[0].is_some(),
            "tube_1_visible":self.slots[1].is_some(),
            "tube_2_visible":self.slots[2].is_some(),
            "tube_3_visible":self.slots[3].is_some(),
            "running":self.running,
            "status":if self.opened {"open"}else if self.running {"running"}else if self.rpm>0.0 {"decelerating"}else{"idle"}})
    }
}
