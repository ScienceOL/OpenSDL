//! Original PH14 electrode teaching model. Synthetic buffer fixtures, first-order response.
use super::{thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PHConfig {
    pub model_id: String,
    pub response_s: f64,
    pub probe_travel_m: f64,
}
impl PHConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || [self.response_s, self.probe_travel_m]
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("invalid pH parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct PHState {
    pub config: PHConfig,
    powered: bool,
    immersed: bool,
    buffer: f64,
    reading: f64,
    calibrated: bool,
}
impl PHState {
    pub fn new(config: PHConfig) -> Self {
        Self {
            config,
            powered: false,
            immersed: false,
            buffer: 7.0,
            reading: 7.0,
            calibrated: false,
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
            "set_buffer" => {
                let ph = thermal::number(params, "ph", 4.0, 10.0)?;
                if ![4.0, 7.0, 10.0].contains(&ph) {
                    return Err("INVALID_PARAMS".into());
                }
                if self.immersed {
                    return Err("RAISE_PROBE_FIRST".into());
                }
                self.buffer = ph;
            }
            "power" => {
                thermal::empty(params)?;
                self.powered = !self.powered;
            }
            "probe" => {
                thermal::empty(params)?;
                self.immersed = !self.immersed;
            }
            "calibrate" => {
                thermal::empty(params)?;
                if !self.powered
                    || !self.immersed
                    || self.buffer != 7.0
                    || (self.reading - self.buffer).abs() > 0.01
                {
                    return Err("WAIT_FOR_BUFFER_7".into());
                }
                self.calibrated = true;
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
        if !self.powered || !self.immersed {
            return false;
        }
        let old = self.reading;
        self.reading =
            self.buffer + (self.reading - self.buffer) * (-dt / self.config.response_s).exp();
        if (self.reading - self.buffer).abs() < 1e-8 {
            self.reading = self.buffer;
        }
        old != self.reading
    }
    pub fn values(&self) -> Value {
        let available = self.powered && self.immersed;
        json!({
            "ph":available.then_some(self.reading),
            "buffer_ph":self.buffer,
            "powered":self.powered,
            "calibrated":self.calibrated,
            "probe_m":if self.immersed {-self.config.probe_travel_m}else{0.0},
            "stable":available&&(self.reading-self.buffer).abs()<0.01,
            "status":if !self.powered {"off"}else if !self.immersed {"dry_probe"}else if (self.reading-self.buffer).abs()<0.01 {"stable"}else{"settling"}})
    }
}
