//! Synthetic optical acquisition; no measured spectral data or analytical accuracy claim.
use super::{thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UvVisConfig {
    pub model_id: String,
    pub warmup_s: f64,
    pub wavelength_min_nm: f64,
    pub wavelength_max_nm: f64,
    pub path_m: f64,
    pub peak_nm: f64,
    pub width_nm: f64,
    pub peak_absorbance: f64,
    pub baseline_absorbance: f64,
    pub opening_rad: f64,
    pub blank_id: String,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl UvVisConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.blank_id.is_empty()
            || self.sample_id.is_empty()
            || self.blank_id == self.sample_id
            || [
                self.warmup_s,
                self.wavelength_min_nm,
                self.wavelength_max_nm,
                self.path_m,
                self.peak_nm,
                self.width_nm,
                self.peak_absorbance,
                self.baseline_absorbance,
                self.opening_rad,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.wavelength_min_nm >= self.wavelength_max_nm
            || self.peak_nm < self.wavelength_min_nm
            || self.peak_nm > self.wavelength_max_nm
            || self.path_m > 0.1
            || (self.baseline_absorbance + self.peak_absorbance) * self.path_m / 0.010 > 6.0
        {
            return Err("invalid synthetic spectrophotometer parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cuvette {
    Blank,
    Sample,
}
#[derive(Debug, Clone)]
pub struct UvVisState {
    pub config: UvVisConfig,
    powered: bool,
    opened: bool,
    cuvette: Option<Cuvette>,
    warmup_remaining: f64,
    wavelength: f64,
    reference: Option<f64>,
    absorbance: Option<f64>,
}
impl UvVisState {
    pub fn new(config: UvVisConfig) -> Self {
        Self {
            wavelength: config.peak_nm,
            warmup_remaining: config.warmup_s,
            config,
            powered: false,
            opened: false,
            cuvette: None,
            reference: None,
            absorbance: None,
        }
    }
    fn require_ready(&self) -> Result<(), String> {
        if !self.powered {
            return Err("POWER_ON_FIRST".into());
        }
        if self.warmup_remaining > 0.0 {
            return Err("WAIT_FOR_LAMP".into());
        }
        if self.opened {
            return Err("CLOSE_LID_FIRST".into());
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
            "power" => {
                thermal::empty(params)?;
                self.powered = !self.powered;
                self.warmup_remaining = self.config.warmup_s;
                self.reference = None;
                self.absorbance = None;
            }
            "lid" => {
                thermal::empty(params)?;
                self.opened = !self.opened;
                self.absorbance = None;
            }
            "set_wavelength" => {
                self.wavelength = thermal::number(
                    params,
                    "wavelength_nm",
                    self.config.wavelength_min_nm,
                    self.config.wavelength_max_nm,
                )?;
                self.reference = None;
                self.absorbance = None;
            }
            "load_cuvette" => {
                let object = params
                    .as_object()
                    .filter(|o| o.len() == 1)
                    .ok_or("INVALID_PARAMS")?;
                let cuvette = match object.get("kind").and_then(Value::as_str) {
                    Some("blank") => Cuvette::Blank,
                    Some("sample") => Cuvette::Sample,
                    _ => return Err("INVALID_PARAMS".into()),
                };
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                if self.cuvette.is_some() {
                    return Err("CUVETTE_OCCUPIED".into());
                }
                let id = match cuvette {
                    Cuvette::Blank => &self.config.blank_id,
                    Cuvette::Sample => &self.config.sample_id,
                };
                samples.claim(id, device, self.config.sample_radius_m)?;
                self.cuvette = Some(cuvette);
                self.absorbance = None;
            }
            "unload_cuvette" => {
                thermal::empty(params)?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                let id = match self.cuvette {
                    Some(Cuvette::Blank) => &self.config.blank_id,
                    Some(Cuvette::Sample) => &self.config.sample_id,
                    None => return Err("LOAD_CUVETTE_FIRST".into()),
                };
                samples.release(id, device)?;
                self.cuvette = None;
                self.absorbance = None;
            }
            "blank" => {
                thermal::empty(params)?;
                self.require_ready()?;
                if self.cuvette != Some(Cuvette::Blank) {
                    return Err("LOAD_BLANK_FIRST".into());
                }
                // Unit synthetic incident intensity. No lamp spectrum is being inferred.
                self.reference = Some(1.0);
                self.absorbance = Some(0.0);
            }
            "measure" => {
                thermal::empty(params)?;
                self.require_ready()?;
                if self.cuvette != Some(Cuvette::Sample) {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                let reference = self.reference.ok_or("BLANK_FIRST")?;
                let c = &self.config;
                let a = (c.baseline_absorbance
                    + c.peak_absorbance
                        * (-0.5 * ((self.wavelength - c.peak_nm) / c.width_nm).powi(2)).exp())
                    * c.path_m
                    / 0.010;
                let intensity = 10.0_f64.powf(-a);
                self.absorbance = Some(-(intensity / reference).log10());
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
        if !self.powered {
            return false;
        }
        let before = self.warmup_remaining;
        self.warmup_remaining = (before - dt).max(0.0);
        before != self.warmup_remaining
    }
    pub fn values(&self) -> Value {
        json!({
            "wavelength_nm":self.wavelength,
            "wavelength_dial_rad":(self.wavelength-self.config.wavelength_min_nm)/(self.config.wavelength_max_nm-self.config.wavelength_min_nm)*std::f64::consts::TAU*0.75,
            "absorbance":self.absorbance,
            "transmittance_pct":self.absorbance.map(|a|100.0*10.0_f64.powf(-a)),
            "powered":self.powered,
            "warmup_remaining_s":self.warmup_remaining,
            "blanked":self.reference.is_some(),
            "cuvette_visible":self.cuvette.is_some(),
            "cuvette_kind":self.cuvette.map(|v|match v{Cuvette::Blank=>"blank",Cuvette::Sample=>"sample"}),
            "lid_rad":if self.opened{self.config.opening_rad}else{0.0},
            "status":if !self.powered{"off"}else if self.warmup_remaining>0.0{"warming"}else if self.opened{"open"}else if self.absorbance.is_some(){"measured"}else{"ready"}
        })
    }
}
