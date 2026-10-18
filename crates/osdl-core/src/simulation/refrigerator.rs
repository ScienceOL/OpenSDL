//! Original cabinet: proportional cooling and separate compartment/sample energy stores.
use super::{
    thermal::{self, ThermalPair},
    SampleInventory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefrigeratorConfig {
    pub model_id: String,
    pub ambient_c: f64,
    pub min_c: f64,
    pub max_c: f64,
    pub cooling_w: f64,
    pub chamber_capacity_j_k: f64,
    pub sample_capacity_j_k: f64,
    pub exchange_w_k: f64,
    pub loss_w_k: f64,
    pub opening_rad: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl RefrigeratorConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || !self.ambient_c.is_finite()
            || !(15.0..40.0).contains(&self.ambient_c)
            || self.min_c < 1.0
            || self.max_c <= self.min_c
            || self.max_c >= self.ambient_c
            || self.cooling_w > 1000.0
            || [
                self.min_c,
                self.max_c,
                self.cooling_w,
                self.chamber_capacity_j_k,
                self.sample_capacity_j_k,
                self.exchange_w_k,
                self.loss_w_k,
                self.opening_rad,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || (self.exchange_w_k + 8.0 * self.loss_w_k) / self.chamber_capacity_j_k > 10.0
            || (self.exchange_w_k + 0.1) / self.sample_capacity_j_k > 10.0
        {
            return Err("invalid refrigerator parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct RefrigeratorState {
    pub config: RefrigeratorConfig,
    thermal: ThermalPair,
    opened: bool,
    loaded: bool,
    running: bool,
    target: f64,
}
impl RefrigeratorState {
    pub fn new(config: RefrigeratorConfig) -> Self {
        Self {
            thermal: ThermalPair::new(config.ambient_c),
            target: 4.0_f64.clamp(config.min_c, config.max_c),
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
                    self.config.min_c,
                    self.config.max_c,
                )?
            }
            "lid" => {
                thermal::empty(params)?;
                self.opened = !self.opened;
            }
            "load_sample" => {
                thermal::empty(params)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                self.loaded = true;
                self.thermal.liquid_c = self.config.ambient_c;
            }
            "unload_sample" => {
                thermal::empty(params)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
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
                if self.running
                    || self.thermal.plate_c < self.config.ambient_c - 2.0
                    || (self.loaded && self.thermal.liquid_c < self.config.ambient_c - 2.0)
                {
                    return Err("WAIT_FOR_AMBIENT_STOP".into());
                }
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let before = (self.thermal.plate_c, self.thermal.liquid_c);
        let c = &self.config;
        let enabled = self.running && !self.opened;
        let target = self.target;
        self.thermal.advance(
            dt,
            |temperature| {
                if enabled {
                    -((temperature - target) * 20.0).clamp(0.0, c.cooling_w)
                } else {
                    0.0
                }
            },
            [c.chamber_capacity_j_k, c.sample_capacity_j_k],
            [
                c.exchange_w_k,
                c.loss_w_k * if self.opened { 8.0 } else { 1.0 },
                0.1,
            ],
            c.ambient_c,
            self.loaded,
        );
        before != (self.thermal.plate_c, self.thermal.liquid_c)
    }
    pub fn values(&self) -> Value {
        let cooling = self.running && !self.opened && self.thermal.plate_c > self.target;
        json!({
            "temperature_c":self.thermal.plate_c,
            "sample_c":self.loaded.then_some(self.thermal.liquid_c),
            "target_c":self.target,
            "lid_rad":if self.opened{self.config.opening_rad}else{0.0},
            "temperature_dial_rad":(self.target-self.config.min_c)/(self.config.max_c-self.config.min_c)*std::f64::consts::TAU*0.75,
            "sample_visible":self.loaded,
            "running":self.running,
            "cooling":cooling,
            "status":if self.opened{"open"}else if cooling{"cooling"}else{"idle"}
        })
    }
}
