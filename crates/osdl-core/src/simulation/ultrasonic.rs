//! Original timed ultrasonic bath: declared thermal input, not cavitation/cleaning efficacy.
use super::{
    thermal::{self, ThermalPair},
    SampleInventory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UltrasonicConfig {
    pub model_id: String,
    pub ambient_c: f64,
    pub max_c: f64,
    pub heater_w: f64,
    pub ultrasound_thermal_w: f64,
    pub tank_capacity_j_k: f64,
    pub water_capacity_j_k: f64,
    pub sample_capacity_j_k: f64,
    pub exchange_w_k: f64,
    pub loss_w_k: f64,
    pub min_fill: f64,
    pub max_duration_s: f64,
    pub opening_rad: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl UltrasonicConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || !self.ambient_c.is_finite()
            || !(0.0..40.0).contains(&self.ambient_c)
            || self.max_c <= self.ambient_c
            || self.max_c > 80.0
            || self.max_duration_s <= 1.0
            || !(0.1..=1.0).contains(&self.min_fill)
            || [
                self.max_c,
                self.heater_w,
                self.ultrasound_thermal_w,
                self.tank_capacity_j_k,
                self.water_capacity_j_k,
                self.sample_capacity_j_k,
                self.exchange_w_k,
                self.loss_w_k,
                self.max_duration_s,
                self.opening_rad,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || (self.exchange_w_k + self.loss_w_k * 5.0) / self.tank_capacity_j_k > 10.0
            || (self.exchange_w_k + 0.1) / self.sample_capacity_j_k > 10.0
        {
            return Err("invalid ultrasonic bath parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct UltrasonicState {
    pub config: UltrasonicConfig,
    thermal: ThermalPair,
    fill: f64,
    loaded: bool,
    opened: bool,
    valve_open: bool,
    running: bool,
    target_c: f64,
    duration_s: f64,
    remaining_s: f64,
}
impl UltrasonicState {
    pub fn new(config: UltrasonicConfig) -> Self {
        Self {
            thermal: ThermalPair::new(config.ambient_c),
            target_c: config.ambient_c,
            duration_s: config.max_duration_s.min(120.0),
            remaining_s: 0.0,
            config,
            fill: 0.0,
            loaded: false,
            opened: false,
            valve_open: false,
            running: false,
        }
    }
    fn change_fill(&mut self, fill: f64) -> Result<(), String> {
        if self.running {
            return Err("STOP_FIRST".into());
        }
        if self.thermal.plate_c > 45.0 {
            return Err("WAIT_FOR_COOL_STOP".into());
        }
        if self.loaded {
            return Err("UNLOAD_BASKET_FIRST".into());
        }
        if fill > self.fill {
            let before = self.config.tank_capacity_j_k + self.fill * self.config.water_capacity_j_k;
            let added = (fill - self.fill) * self.config.water_capacity_j_k;
            self.thermal.plate_c =
                (before * self.thermal.plate_c + added * self.config.ambient_c) / (before + added);
        }
        self.fill = fill;
        Ok(())
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
                self.target_c = thermal::number(
                    params,
                    "temperature_c",
                    self.config.ambient_c,
                    self.config.max_c,
                )?
            }
            "set_duration" => {
                let duration = thermal::number(params, "seconds", 1.0, self.config.max_duration_s)?;
                if self.running {
                    return Err("STOP_FIRST".into());
                }
                self.duration_s = duration;
                self.remaining_s = duration;
            }
            "set_fill" => {
                let fill = thermal::number(params, "fraction", 0.0, 1.0)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                self.change_fill(fill)?;
                self.valve_open = false;
            }
            "drain" => {
                thermal::empty(params)?;
                self.change_fill(0.0)?;
                self.valve_open = true;
            }
            "lid" => {
                thermal::empty(params)?;
                if self.running {
                    return Err("STOP_FIRST".into());
                }
                self.opened = !self.opened;
            }
            "load_basket" | "unload_basket" => {
                thermal::empty(params)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                if self.running {
                    return Err("STOP_FIRST".into());
                }
                if action == "load_basket" {
                    samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                    self.loaded = true;
                } else {
                    if self.thermal.liquid_c > 45.0 {
                        return Err("WAIT_FOR_COOL_STOP".into());
                    }
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                }
            }
            "start" => {
                thermal::empty(params)?;
                if self.running {
                    return Err("STOP_FIRST".into());
                }
                if self.opened {
                    return Err("CLOSE_LID_FIRST".into());
                }
                if self.fill < self.config.min_fill || self.valve_open {
                    return Err("FILL_TANK_FIRST".into());
                }
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.remaining_s = self.duration_s;
                self.running = true;
            }
            "stop" => {
                thermal::empty(params)?;
                self.running = false;
            }
            "reset" => {
                thermal::empty(params)?;
                if self.running || self.thermal.plate_c > 45.0 {
                    return Err("WAIT_FOR_COOL_STOP".into());
                }
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    fn thermal_step(&mut self, dt: f64, enabled: bool) {
        if dt <= 0.0 {
            return;
        }
        let c = &self.config;
        let target = self.target_c;
        self.thermal.advance(
            dt,
            |temperature| {
                if enabled && temperature < 95.0 {
                    c.ultrasound_thermal_w + ((target - temperature) * 30.0).clamp(0.0, c.heater_w)
                } else {
                    0.0
                }
            },
            [
                c.tank_capacity_j_k + self.fill * c.water_capacity_j_k,
                c.sample_capacity_j_k,
            ],
            [
                c.exchange_w_k,
                c.loss_w_k * if self.opened { 5.0 } else { 1.0 },
                0.1,
            ],
            c.ambient_c,
            self.loaded && self.fill >= c.min_fill,
        );
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let before = (
            self.thermal.plate_c,
            self.thermal.liquid_c,
            self.remaining_s,
            self.running,
        );
        let active = if self.running {
            dt.min(self.remaining_s)
        } else {
            0.0
        };
        self.thermal_step(active, true);
        self.remaining_s -= active;
        if self.remaining_s <= 0.0 {
            self.running = false;
        }
        self.thermal_step(dt - active, false);
        before
            != (
                self.thermal.plate_c,
                self.thermal.liquid_c,
                self.remaining_s,
                self.running,
            )
    }
    pub fn values(&self) -> Value {
        let status = if self.running {
            "sonicating"
        } else if self.opened {
            "open"
        } else if self.thermal.plate_c > 45.0 {
            "hot"
        } else {
            "idle"
        };
        json!({
            "temperature_c": self.thermal.plate_c,
            "sample_c": self.loaded.then_some(self.thermal.liquid_c),
            "target_c": self.target_c,
            "fill_fraction": self.fill,
            "water_visible": self.fill>0.0,
            "sample_visible": self.loaded,
            "duration_s": self.duration_s,
            "remaining_s": self.remaining_s,
            "running": self.running,
            "hot": self.thermal.plate_c>45.0,
            "lid_rad": if self.opened {self.config.opening_rad}else{0.0},
            "drain_rad": if self.valve_open {std::f64::consts::FRAC_PI_2}else{0.0},
            "temperature_dial_rad": (self.target_c-self.config.ambient_c)/(self.config.max_c-self.config.ambient_c)*std::f64::consts::TAU*0.75,
            "duration_dial_rad": (self.duration_s-1.0)/(self.config.max_duration_s-1.0)*std::f64::consts::TAU*0.75,
            "status": status
        })
    }
}
