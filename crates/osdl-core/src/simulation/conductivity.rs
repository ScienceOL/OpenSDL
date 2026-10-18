//! Synthetic two-electrode conductivity response and explicit 25 C compensation.
use super::{thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConductivityConfig {
    pub model_id: String,
    pub response_s: f64,
    pub probe_travel_m: f64,
    pub standard_25_us_cm: f64,
    pub sample_25_us_cm: f64,
    pub temperature_coefficient: f64,
    pub probe_gain: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl ConductivityConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.response_s,
                self.probe_travel_m,
                self.standard_25_us_cm,
                self.sample_25_us_cm,
                self.temperature_coefficient,
                self.probe_gain,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.standard_25_us_cm > 1e6
            || self.sample_25_us_cm > 1e6
            || self.temperature_coefficient >= 0.05
            || !(0.5..=1.5).contains(&self.probe_gain)
        {
            return Err("invalid conductivity parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Solution {
    Standard,
    Sample,
}
#[derive(Debug, Clone)]
pub struct ConductivityState {
    pub config: ConductivityConfig,
    solution: Solution,
    temperature_c: f64,
    powered: bool,
    immersed: bool,
    loaded: bool,
    raw: f64,
    correction: f64,
    calibrated: bool,
}
impl ConductivityState {
    pub fn new(config: ConductivityConfig) -> Self {
        Self {
            config,
            solution: Solution::Sample,
            temperature_c: 25.0,
            powered: false,
            immersed: false,
            loaded: false,
            raw: 0.0,
            correction: 1.0,
            calibrated: false,
        }
    }
    fn factor(&self) -> f64 {
        1.0 + self.config.temperature_coefficient * (self.temperature_c - 25.0)
    }
    fn target_raw(&self) -> f64 {
        let reference = match self.solution {
            Solution::Standard => self.config.standard_25_us_cm,
            Solution::Sample => self.config.sample_25_us_cm,
        };
        reference * self.factor() * self.config.probe_gain
    }
    fn available(&self) -> bool {
        self.powered && self.immersed && self.loaded
    }
    fn stable(&self) -> bool {
        self.available()
            && self.raw > 0.0
            && (self.raw - self.target_raw()).abs() < self.target_raw() * 1e-4
    }
    pub fn apply(
        &mut self,
        device: &str,
        action: &str,
        params: &Value,
        samples: &mut SampleInventory,
    ) -> Result<(), String> {
        match action {
            "set_solution" => {
                let object = params
                    .as_object()
                    .filter(|o| o.len() == 1)
                    .ok_or("INVALID_PARAMS")?;
                let solution = match object.get("solution").and_then(Value::as_str) {
                    Some("standard") => Solution::Standard,
                    Some("sample") => Solution::Sample,
                    _ => return Err("INVALID_PARAMS".into()),
                };
                if self.immersed {
                    return Err("RAISE_PROBE_FIRST".into());
                }
                self.solution = solution;
                self.raw = 0.0;
            }
            "set_temperature" => {
                self.temperature_c = thermal::number(params, "temperature_c", 5.0, 40.0)?;
            }
            "power" => {
                thermal::empty(params)?;
                self.powered = !self.powered;
            }
            "probe" => {
                thermal::empty(params)?;
                if !self.immersed && !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.immersed = !self.immersed;
                if !self.immersed {
                    self.raw = 0.0;
                }
            }
            "load_sample" | "unload_sample" => {
                thermal::empty(params)?;
                if self.immersed {
                    return Err("RAISE_PROBE_FIRST".into());
                }
                if action == "load_sample" {
                    samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                    self.loaded = true;
                } else {
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                }
                self.raw = 0.0;
            }
            "calibrate" => {
                thermal::empty(params)?;
                if self.solution != Solution::Standard || !self.stable() {
                    return Err("WAIT_FOR_CONDUCTIVITY_STANDARD".into());
                }
                self.correction = self.config.standard_25_us_cm * self.factor() / self.raw;
                self.calibrated = true;
            }
            "reset" => {
                thermal::empty(params)?;
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        if !self.available() {
            return false;
        }
        let before = self.raw;
        let target = self.target_raw();
        self.raw = target + (self.raw - target) * (-dt / self.config.response_s).exp();
        before != self.raw
    }
    pub fn values(&self) -> Value {
        let available = self.available();
        json!({
            "conductivity_us_cm":available.then_some(self.raw*self.correction),
            "conductivity_25_us_cm":available.then_some(self.raw*self.correction/self.factor()),
            "temperature_c":self.temperature_c,
            "solution":match self.solution{Solution::Standard=>"standard",Solution::Sample=>"sample"},
            "probe_m":if self.immersed{-self.config.probe_travel_m}else{0.0},
            "powered":self.powered,
            "sample_visible":self.loaded,
            "calibrated":self.calibrated,
            "stable":self.stable(),
            "status":if !self.powered{"off"}else if !available{"dry_probe"}else if self.stable(){"stable"}else{"settling"}
        })
    }
}
