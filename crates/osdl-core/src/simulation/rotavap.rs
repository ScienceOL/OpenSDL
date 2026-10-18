//! Original rotary evaporator drive/bath/vacuum; mass transfer is explicitly unavailable.
use super::{
    motor::Motor,
    pressure::pressure_after,
    thermal::{self, ThermalPair},
    SampleInventory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotavapConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub lift_m: f64,
    pub immersion_max_m: f64,
    pub ambient_c: f64,
    pub max_c: f64,
    pub heater_w: f64,
    pub bath_capacity_j_k: f64,
    pub sample_capacity_j_k: f64,
    pub exchange_w_k: f64,
    pub loss_w_k: f64,
    pub ambient_kpa: f64,
    pub ultimate_kpa: f64,
    pub chamber_l: f64,
    pub pumping_l_s: f64,
    pub leak_l_s: f64,
    pub sample_ml: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl RotavapConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.max_rpm,
                self.slew_rpm_s,
                self.lift_m,
                self.immersion_max_m,
                self.ambient_c,
                self.max_c,
                self.heater_w,
                self.bath_capacity_j_k,
                self.sample_capacity_j_k,
                self.exchange_w_k,
                self.loss_w_k,
                self.ambient_kpa,
                self.ultimate_kpa,
                self.chamber_l,
                self.pumping_l_s,
                self.leak_l_s,
                self.sample_ml,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.immersion_max_m >= self.lift_m
            || self.max_rpm > 500.0
            || self.max_c > 80.0
            || self.max_c <= self.ambient_c
            || !(0.0..40.0).contains(&self.ambient_c)
            || self.heater_w > 1000.0
            || (self.exchange_w_k + 5.0 * self.loss_w_k) / self.bath_capacity_j_k > 10.0
            || (self.exchange_w_k + 0.1) / self.sample_capacity_j_k > 10.0
            || !(20.0..=120.0).contains(&self.ambient_kpa)
            || self.ultimate_kpa >= self.ambient_kpa
            || !(0.01..=100.0).contains(&self.chamber_l)
            || self.pumping_l_s > 10.0
            || self.leak_l_s > 10.0
        {
            return Err("invalid rotary evaporator parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct RotavapState {
    pub config: RotavapConfig,
    motor: Motor,
    thermal: ThermalPair,
    pressure: f64,
    height: f64,
    loaded: bool,
    clamped: bool,
    heater: bool,
    pump: bool,
    vent: bool,
    target: f64,
}
impl RotavapState {
    pub fn new(config: RotavapConfig) -> Self {
        Self {
            motor: Motor::new(config.max_rpm, config.slew_rpm_s),
            thermal: ThermalPair::new(config.ambient_c),
            pressure: config.ambient_kpa,
            height: config.lift_m,
            target: config.ambient_c,
            config,
            loaded: false,
            clamped: false,
            heater: false,
            pump: false,
            vent: false,
        }
    }
    fn require_vented_stopped(&self) -> Result<(), String> {
        self.motor.require_stopped()?;
        if self.pump || self.config.ambient_kpa - self.pressure > 0.5 {
            return Err("VENT_FIRST".into());
        }
        Ok(())
    }
    fn require_immersed(&self) -> Result<(), String> {
        if !self.loaded {
            return Err("LOAD_FLASK_FIRST".into());
        }
        if !self.clamped {
            return Err("CLAMP_FLASK_FIRST".into());
        }
        if self.height > self.config.immersion_max_m {
            return Err("LOWER_FLASK_FIRST".into());
        }
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
            "set_speed" => self.motor.set_speed(params)?,
            "set_temperature" => {
                self.target = thermal::number(
                    params,
                    "temperature_c",
                    self.config.ambient_c,
                    self.config.max_c,
                )?
            }
            "load_flask" | "unload_flask" => {
                thermal::empty(params)?;
                self.require_vented_stopped()?;
                if self.height < self.config.lift_m {
                    return Err("RAISE_FLASK_FIRST".into());
                }
                if self.clamped {
                    return Err("UNCLAMP_FLASK_FIRST".into());
                }
                if action == "load_flask" {
                    samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                    self.loaded = true;
                    self.thermal.liquid_c = self.config.ambient_c;
                } else {
                    if self.thermal.liquid_c > 45.0 {
                        return Err("WAIT_FOR_COOL_STOP".into());
                    }
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                }
            }
            "clamp" => {
                thermal::empty(params)?;
                self.require_vented_stopped()?;
                if !self.loaded {
                    return Err("LOAD_FLASK_FIRST".into());
                }
                self.clamped = !self.clamped;
            }
            "set_height" | "lift" => {
                let height = if action == "lift" {
                    thermal::empty(params)?;
                    if self.height > 0.0 {
                        0.0
                    } else {
                        self.config.lift_m
                    }
                } else {
                    thermal::number(params, "height_m", 0.0, self.config.lift_m)?
                };
                self.require_vented_stopped()?;
                if self.loaded && !self.clamped {
                    return Err("CLAMP_FLASK_FIRST".into());
                }
                self.height = height;
                if self.height > self.config.immersion_max_m {
                    self.heater = false;
                }
            }
            "start" => {
                thermal::empty(params)?;
                self.require_immersed()?;
                self.motor.start()?;
            }
            "heat" => {
                thermal::empty(params)?;
                if !self.heater {
                    self.require_immersed()?;
                }
                self.heater = !self.heater;
            }
            "start_vacuum" => {
                thermal::empty(params)?;
                self.require_immersed()?;
                if self.vent {
                    return Err("CLOSE_VENT_FIRST".into());
                }
                self.pump = true;
            }
            "stop_vacuum" => {
                thermal::empty(params)?;
                self.pump = false;
            }
            "stop" => {
                thermal::empty(params)?;
                self.motor.running = false;
                self.heater = false;
                self.pump = false;
            }
            "vent" => {
                thermal::empty(params)?;
                self.vent = !self.vent;
                if self.vent {
                    self.pump = false;
                }
            }
            "reset" => {
                thermal::empty(params)?;
                self.require_vented_stopped()?;
                if self.heater
                    || self.thermal.plate_c > self.config.ambient_c + 2.0
                    || (self.loaded && self.thermal.liquid_c > self.config.ambient_c + 2.0)
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
        let before = (
            self.motor.rpm,
            self.motor.angle,
            self.thermal.plate_c,
            self.thermal.liquid_c,
            self.pressure,
        );
        self.motor.advance(dt);
        let c = &self.config;
        let target = self.target;
        let heater = self.heater;
        self.thermal.advance(
            dt,
            |t| {
                if heater {
                    ((target - t) * 30.0).clamp(0.0, c.heater_w)
                } else {
                    0.0
                }
            },
            [c.bath_capacity_j_k, c.sample_capacity_j_k],
            [c.exchange_w_k, c.loss_w_k, 0.1],
            c.ambient_c,
            self.loaded && self.height <= c.immersion_max_m,
        );
        self.pressure = pressure_after(
            self.pressure,
            dt,
            if self.pump { c.pumping_l_s } else { 0.0 },
            if self.vent || !self.loaded {
                2.0
            } else {
                c.leak_l_s
            },
            c.ultimate_kpa,
            c.ambient_kpa,
            c.chamber_l,
        );
        before
            != (
                self.motor.rpm,
                self.motor.angle,
                self.thermal.plate_c,
                self.thermal.liquid_c,
                self.pressure,
            )
    }
    pub fn motion(&self) -> Value {
        json!({"flask_rad_s": self.motor.motion(1.0, None)})
    }
    pub fn values(&self) -> Value {
        json!({
            "rpm":self.motor.rpm,
            "target_rpm":self.motor.target,
            "flask_rad":self.motor.angle,
            "flask_rad_s":self.motor.velocity(),
            "speed_dial_rad":self.motor.dial(),
            "temperature_c":self.thermal.plate_c,
            "sample_c":self.loaded.then_some(self.thermal.liquid_c),
            "target_c":self.target,
            "temperature_dial_rad":(self.target-self.config.ambient_c)/(self.config.max_c-self.config.ambient_c)*std::f64::consts::TAU*0.75,
            "pressure_kpa":self.pressure,
            "carriage_m":self.height,
            "clamp_rad":if self.clamped{0.0}else{0.6},
            "clamped":self.clamped,
            "vent_rad":if self.vent{std::f64::consts::FRAC_PI_2}else{0.0},
            "flask_visible":self.loaded,
            "liquid_visible":self.loaded,
            "sample_ml":self.loaded.then_some(self.config.sample_ml),
            "evaporation_available":false,
            "heater_on":self.heater,
            "vacuum_on":self.pump,
            "running":self.motor.running||self.heater||self.pump,
            "hot":self.thermal.plate_c>45.0||self.thermal.liquid_c>45.0,
            "status":if self.vent{"venting"}else if self.motor.running||self.heater||self.pump{"running"}else if self.motor.rpm>0.0{"decelerating"}else{"idle"}
        })
    }
}
