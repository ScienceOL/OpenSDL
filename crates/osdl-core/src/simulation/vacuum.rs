//! Original VP20 pump plus sealed test chamber, isothermal ideal gas fixture.
use super::{pressure::pressure_after, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VacuumConfig {
    pub model_id: String,
    pub ambient_kpa: f64,
    pub ultimate_kpa: f64,
    pub chamber_l: f64,
    pub pumping_l_s: f64,
    pub leak_l_s: f64,
}
impl VacuumConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || [
                self.ambient_kpa,
                self.ultimate_kpa,
                self.chamber_l,
                self.pumping_l_s,
                self.leak_l_s,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.ultimate_kpa >= self.ambient_kpa
            || !(20.0..=120.0).contains(&self.ambient_kpa)
            || !(0.001..=1000.0).contains(&self.chamber_l)
            || !(1e-6..=100.0).contains(&self.pumping_l_s)
            || !(1e-9..=100.0).contains(&self.leak_l_s)
        {
            return Err("invalid vacuum parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct VacuumState {
    pub config: VacuumConfig,
    pressure: f64,
    vent: bool,
    running: bool,
}
impl VacuumState {
    pub fn new(config: VacuumConfig) -> Self {
        Self {
            pressure: config.ambient_kpa,
            config,
            vent: false,
            running: false,
        }
    }
    pub fn apply(
        &mut self,
        _device: &str,
        action: &str,
        params: &Value,
        _samples: &mut SampleInventory,
    ) -> Result<(), String> {
        thermal::empty(params)?;
        match action {
            "start" => {
                if self.vent {
                    return Err("CLOSE_VENT_FIRST".into());
                }
                self.running = true;
            }
            "stop" => self.running = false,
            "vent" => {
                self.vent = !self.vent;
                if self.vent {
                    self.running = false;
                }
            }
            "reset" => *self = Self::new(self.config.clone()),
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let old = self.pressure;
        let c = &self.config;
        let pump = if self.running { c.pumping_l_s } else { 0.0 };
        let leak = if self.vent { 2.0 } else { c.leak_l_s };
        self.pressure = pressure_after(
            self.pressure,
            dt,
            pump,
            leak,
            c.ultimate_kpa,
            c.ambient_kpa,
            c.chamber_l,
        );
        old != self.pressure
    }
    pub fn values(&self) -> Value {
        json!({
            "pressure_kpa":self.pressure,
            "running":self.running,
            "vent_rad":if self.vent {std::f64::consts::FRAC_PI_2}else{0.0},
            "status":if self.vent {"venting"}else if self.running {"pumping"}else{"idle"}})
    }
}
