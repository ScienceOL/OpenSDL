//! Original HS150 water fixture: two thermal nodes, bounded motor slew, no CFD/magnetic coupling.
use super::{
    motor,
    thermal::{self, ThermalPair},
    SampleInventory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StirrerConfig {
    pub model_id: String,
    pub ambient_c: f64,
    pub max_plate_c: f64,
    pub max_loaded_plate_c: f64,
    pub max_rpm: f64,
    pub heater_w: f64,
    pub plate_capacity_j_k: f64,
    pub liquid_capacity_j_k: f64,
    pub plate_liquid_w_k: f64,
    pub plate_air_w_k: f64,
    pub liquid_air_w_k: f64,
    pub slew_rpm_s: f64,
    pub vessel_radius_m: f64,
}
impl StirrerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || !self.ambient_c.is_finite()
            || !(0.0..40.0).contains(&self.ambient_c)
            || self.max_plate_c <= self.ambient_c
            || self.max_plate_c > 200.0
            || self.max_loaded_plate_c <= self.ambient_c
            || self.max_loaded_plate_c > self.max_plate_c
            || self.max_loaded_plate_c > 95.0
            || self.plate_capacity_j_k < 1.0
            || self.liquid_capacity_j_k < 1.0
            || self.heater_w > 5000.0
            || self.max_rpm > 30000.0
            || (self.plate_liquid_w_k + self.plate_air_w_k) / self.plate_capacity_j_k > 10.0
            || (self.plate_liquid_w_k + self.liquid_air_w_k) / self.liquid_capacity_j_k > 10.0
            || [
                self.max_plate_c,
                self.max_loaded_plate_c,
                self.max_rpm,
                self.heater_w,
                self.plate_capacity_j_k,
                self.liquid_capacity_j_k,
                self.plate_liquid_w_k,
                self.plate_air_w_k,
                self.liquid_air_w_k,
                self.slew_rpm_s,
                self.vessel_radius_m,
            ]
            .iter()
            .any(|n| !n.is_finite() || *n <= 0.0)
        {
            return Err("invalid stirrer parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct StirrerState {
    pub config: StirrerConfig,
    pub thermal: ThermalPair,
    powered: bool,
    heating: bool,
    stirring: bool,
    probe: bool,
    loaded: bool,
    target_c: f64,
    target_rpm: f64,
    rpm: f64,
    angle: f64,
    fault: bool,
}
impl StirrerState {
    pub fn new(config: StirrerConfig) -> Self {
        Self {
            thermal: ThermalPair::new(config.ambient_c),
            target_c: config.ambient_c,
            config,
            powered: false,
            heating: false,
            stirring: false,
            probe: false,
            loaded: false,
            target_rpm: 0.0,
            rpm: 0.0,
            angle: 0.0,
            fault: false,
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
                let target = thermal::number(
                    params,
                    "temperature_c",
                    self.config.ambient_c,
                    self.config.max_plate_c,
                )?;
                if self.loaded && target > self.config.max_loaded_plate_c {
                    return Err("LOADED_WATER_LIMIT".into());
                }
                self.target_c = target;
            }
            "set_speed" => {
                self.target_rpm = thermal::number(params, "rpm", 0.0, self.config.max_rpm)?
            }
            "load_vessel" => {
                thermal::empty(params)?;
                if self.loaded {
                    return Err("VESSEL_ALREADY_LOADED".into());
                }
                if self.thermal.plate_c > self.config.max_loaded_plate_c {
                    return Err("WAIT_FOR_COOL_STOP".into());
                }
                if self.heating && self.target_c > self.config.max_loaded_plate_c {
                    return Err("LOADED_WATER_LIMIT".into());
                }
                samples.claim("water_vessel", device, self.config.vessel_radius_m)?;
                self.loaded = true;
            }
            "unload_vessel" => {
                thermal::empty(params)?;
                if self.heating || self.rpm > 1.0 || self.thermal.liquid_c > 45.0 {
                    return Err("WAIT_FOR_COOL_STOP".into());
                }
                samples.release("water_vessel", device)?;
                self.loaded = false;
                self.stirring = false;
                self.probe = false;
            }
            "probe" => {
                thermal::empty(params)?;
                if !self.loaded {
                    return Err("LOAD_VESSEL_FIRST".into());
                }
                self.probe = !self.probe;
            }
            "power" => {
                thermal::empty(params)?;
                self.powered = !self.powered;
                if !self.powered {
                    self.heating = false;
                    self.stirring = false;
                }
            }
            "heat" => {
                thermal::empty(params)?;
                if !self.powered {
                    return Err("POWER_REQUIRED".into());
                }
                if self.fault {
                    return Err("THERMAL_CUTOFF".into());
                }
                if !self.heating && self.loaded && self.target_c > self.config.max_loaded_plate_c {
                    return Err("LOADED_WATER_LIMIT".into());
                }
                self.heating = !self.heating;
            }
            "stir" => {
                thermal::empty(params)?;
                if !self.powered {
                    return Err("POWER_REQUIRED".into());
                }
                if !self.loaded {
                    return Err("LOAD_VESSEL_FIRST".into());
                }
                self.stirring = !self.stirring;
            }
            "stop" => {
                thermal::empty(params)?;
                self.heating = false;
                self.stirring = false;
            }
            "reset" => {
                thermal::empty(params)?;
                if self.thermal.plate_c > 45.0 || self.rpm > 1.0 {
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
        let previous = (
            self.thermal.plate_c,
            self.thermal.liquid_c,
            self.rpm,
            self.angle,
            self.fault,
            self.heating,
        );
        let enabled = self.powered && self.heating && !self.fault;
        let target = self.target_c;
        let heater_w = self.config.heater_w;
        let power = |plate_c: f64| {
            if enabled {
                ((target - plate_c) * 20.0_f64).clamp(0.0, heater_w)
            } else {
                0.0
            }
        };
        self.thermal.advance(
            dt,
            power,
            [
                self.config.plate_capacity_j_k,
                self.config.liquid_capacity_j_k,
            ],
            [
                self.config.plate_liquid_w_k,
                self.config.plate_air_w_k,
                self.config.liquid_air_w_k,
            ],
            self.config.ambient_c,
            self.loaded,
        );
        if self.thermal.liquid_c >= 98.0 || self.thermal.plate_c > self.config.max_plate_c + 5.0 {
            self.fault = true;
            self.heating = false;
        }
        let (rpm, radians) = motor::advance_rpm(
            self.rpm,
            if self.powered && self.stirring {
                self.target_rpm
            } else {
                0.0
            },
            self.config.slew_rpm_s,
            dt,
        );
        self.rpm = rpm;
        self.angle = (self.angle + radians).rem_euclid(std::f64::consts::TAU);
        // Repeated sampled angles still represent motion (e.g. one turn per tick).
        self.rpm > 0.0
            || previous
                != (
                    self.thermal.plate_c,
                    self.thermal.liquid_c,
                    self.rpm,
                    self.angle,
                    self.fault,
                    self.heating,
                )
    }
    pub fn motion(&self) -> Value {
        let target = if self.powered && self.stirring {
            self.target_rpm
        } else {
            0.0
        };
        json!({"bar_rad_s": motor::profile(target, self.config.slew_rpm_s, 1.0, None)})
    }
    pub fn values(&self) -> Value {
        json!({
            "powered":self.powered,
            "heating":self.heating,
            "stirring":self.stirring,
            "plate_c":self.thermal.plate_c,
            "liquid_c":self.loaded.then_some(self.thermal.liquid_c),
            "probe_c":(self.probe&&self.loaded&&self.powered).then_some(self.thermal.liquid_c),
            "target_c":self.target_c,
            "target_rpm":self.target_rpm,
            "rpm":self.rpm,
            "heat_dial_rad":(self.target_c-self.config.ambient_c)/(self.config.max_plate_c-self.config.ambient_c)*std::f64::consts::TAU*0.75,
            "speed_dial_rad":self.target_rpm/self.config.max_rpm*std::f64::consts::TAU*0.75,
            "bar_rad":self.angle,
            "bar_rad_s":self.rpm*std::f64::consts::TAU/60.0,
            "vessel_visible":self.loaded,
            "probe_visible":self.probe,
            "hot":self.thermal.plate_c>45.0,
            "status":if self.fault {"thermal_cutoff"} else if self.heating {"heating"} else if self.thermal.plate_c>45.0 {"hot"} else if self.stirring {"stirring"} else if self.rpm>0.0 {"decelerating"} else if self.powered {"ready"} else {"off"}})
    }
}
