//! Original water bath and drying oven. Closed thermal chamber, no evaporation or drying chemistry.
use super::{
    thermal::{self, ThermalPair},
    SampleInventory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChamberConfig {
    pub model_id: String,
    pub ambient_c: f64,
    pub max_c: f64,
    pub heater_w: f64,
    pub chamber_capacity_j_k: f64,
    pub sample_capacity_j_k: f64,
    pub exchange_w_k: f64,
    pub loss_w_k: f64,
    pub opening_rad: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl ChamberConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || !self.ambient_c.is_finite()
            || !(0.0..40.0).contains(&self.ambient_c)
            || self.max_c <= self.ambient_c
            || self.max_c > 250.0
            || self.chamber_capacity_j_k < 1.0
            || self.sample_capacity_j_k < 1.0
            || self.heater_w > 5000.0
            || (self.exchange_w_k + 5.0 * self.loss_w_k) / self.chamber_capacity_j_k > 10.0
            || (self.exchange_w_k + 0.1) / self.sample_capacity_j_k > 10.0
            || [
                self.max_c,
                self.heater_w,
                self.chamber_capacity_j_k,
                self.sample_capacity_j_k,
                self.exchange_w_k,
                self.loss_w_k,
                self.opening_rad,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("invalid chamber configuration".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct ChamberState {
    pub config: ChamberConfig,
    pub thermal: ThermalPair,
    opened: bool,
    loaded: bool,
    running: bool,
    target: f64,
}
impl ChamberState {
    pub fn new(config: ChamberConfig) -> Self {
        Self {
            thermal: ThermalPair::new(config.ambient_c),
            target: config.ambient_c,
            config,
            opened: false,
            loaded: false,
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
            "set_temperature" => {
                self.target = thermal::number(
                    params,
                    "temperature_c",
                    self.config.ambient_c,
                    self.config.max_c,
                )?
            }
            "lid" => {
                thermal::empty(params)?;
                self.opened = !self.opened;
                if self.opened {
                    self.running = false;
                }
            }
            "load_sample" => {
                thermal::empty(params)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                self.loaded = true;
            }
            "unload_sample" => {
                thermal::empty(params)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                if self.thermal.liquid_c > 45.0 {
                    return Err("WAIT_FOR_COOL_STOP".into());
                }
                samples.release(&self.config.sample_id, device)?;
                self.loaded = false;
            }
            "start" => {
                thermal::empty(params)?;
                if self.opened {
                    return Err("CLOSE_LID_FIRST".into());
                }
                self.running = true;
            }
            "stop" => {
                thermal::empty(params)?;
                self.running = false;
            }
            "reset" => {
                thermal::empty(params)?;
                if self.thermal.plate_c > 45.0 {
                    return Err("WAIT_FOR_COOL_STOP".into());
                }
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let previous = (self.thermal.plate_c, self.thermal.liquid_c);
        let enabled = self.running;
        let target = self.target;
        let heater_w = self.config.heater_w;
        let power = |plate_c: f64| {
            if enabled {
                ((target - plate_c) * 30.0_f64).clamp(0.0, heater_w)
            } else {
                0.0
            }
        };
        self.thermal.advance(
            dt,
            power,
            [
                self.config.chamber_capacity_j_k,
                self.config.sample_capacity_j_k,
            ],
            [
                self.config.exchange_w_k,
                self.config.loss_w_k * if self.opened { 5.0 } else { 1.0 },
                0.1,
            ],
            self.config.ambient_c,
            self.loaded,
        );
        previous != (self.thermal.plate_c, self.thermal.liquid_c)
    }
    pub fn values(&self) -> Value {
        json!({
            "temperature_c":self.thermal.plate_c,
            "sample_c":self.loaded.then_some(self.thermal.liquid_c),
            "target_c":self.target,
            "temperature_dial_rad":(self.target-self.config.ambient_c)/(self.config.max_c-self.config.ambient_c)*std::f64::consts::TAU*0.75,
            "lid_rad":if self.opened {self.config.opening_rad}else{0.0},
            "sample_visible":self.loaded,
            "running":self.running,
            "hot":self.thermal.plate_c>45.0,
            "status":if self.opened {"open"}else if self.running {"heating"}else if self.thermal.plate_c>45.0 {"hot"}else{"idle"}})
    }
}
