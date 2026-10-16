//! Original SP10 positive-displacement fixture. Conserves source+syringe+receiver volume.
use super::{thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyringeConfig {
    pub model_id: String,
    pub capacity_ml: f64,
    pub max_flow_ml_min: f64,
    pub bore_radius_m: f64,
    pub stroke_m: f64,
}
impl SyringeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || [
                self.capacity_ml,
                self.max_flow_ml_min,
                self.bore_radius_m,
                self.stroke_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.capacity_ml * 1e-6
                > std::f64::consts::PI * self.bore_radius_m.powi(2) * self.stroke_m
        {
            return Err("invalid syringe parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct SyringeState {
    pub config: SyringeConfig,
    remaining: f64,
    receiver: f64,
    source: f64,
    flow: f64,
    running: bool,
}
impl SyringeState {
    pub fn new(config: SyringeConfig) -> Self {
        Self {
            remaining: config.capacity_ml,
            source: config.capacity_ml,
            receiver: 0.0,
            flow: 0.0,
            running: false,
            config,
        }
    }
    pub fn apply(
        &mut self,
        _device: &str,
        action: &str,
        params: &Value,
        _samples: &mut SampleInventory,
    ) -> Result<(), String> {
        match action {
            "set_flow" => {
                self.flow =
                    thermal::number(params, "flow_ml_min", 0.0, self.config.max_flow_ml_min)?;
                if self.flow == 0.0 {
                    self.running = false;
                }
            }
            "start" => {
                thermal::empty(params)?;
                if self.remaining <= 0.0 {
                    return Err("SYRINGE_EMPTY".into());
                }
                if self.flow <= 0.0 {
                    return Err("SET_FLOW_FIRST".into());
                }
                self.running = true;
            }
            "stop" => {
                thermal::empty(params)?;
                self.running = false;
            }
            "refill" => {
                thermal::empty(params)?;
                if self.running {
                    return Err("STOP_FIRST".into());
                }
                let volume = (self.config.capacity_ml - self.remaining).min(self.source);
                if volume <= 0.0 {
                    return Err("INSUFFICIENT_VOLUME".into());
                }
                self.source -= volume;
                self.remaining += volume;
            }
            "reset" => {
                thermal::empty(params)?;
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        if !self.running {
            return false;
        }
        let volume = (self.flow * dt / 60.0).min(self.remaining);
        self.remaining -= volume;
        self.receiver += volume;
        if self.remaining <= 1e-10 {
            self.remaining = 0.0;
            self.running = false;
        }
        volume > 0.0
    }
    pub fn values(&self) -> Value {
        json!({
            "remaining_ml":self.remaining,
            "fill_fraction":self.remaining/self.config.capacity_ml,
            "liquid_visible":self.remaining>0.0,
            "delivered_ml":self.receiver,
            "source_ml":self.source,
            "flow_ml_min":self.flow,
            "plunger_m":(self.config.capacity_ml-self.remaining)*1e-6/(std::f64::consts::PI*self.config.bore_radius_m.powi(2)),
            "flow_dial_rad":self.flow/self.config.max_flow_ml_min*std::f64::consts::TAU*0.75,
            "running":self.running,
            "status":if self.remaining==0.0 {"empty"}else if self.running {"dispensing"}else{"idle"}})
    }
}
