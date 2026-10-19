//! Synthetic pressure-driven liquid transfer, not a particle-separation model.
use super::{pressure::pressure_after, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiltrationConfig {
    pub model_id: String,
    pub ambient_kpa: f64,
    pub ultimate_kpa: f64,
    pub chamber_l: f64,
    pub pumping_l_s: f64,
    pub leak_l_s: f64,
    pub filter_ml_s_kpa: f64,
    pub source_ml: f64,
    pub receiver_capacity_ml: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl FiltrationConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.ambient_kpa,
                self.ultimate_kpa,
                self.chamber_l,
                self.pumping_l_s,
                self.leak_l_s,
                self.filter_ml_s_kpa,
                self.source_ml,
                self.receiver_capacity_ml,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || !(20.0..=120.0).contains(&self.ambient_kpa)
            || self.ultimate_kpa >= self.ambient_kpa
            || !(0.01..=100.0).contains(&self.chamber_l)
            || self.receiver_capacity_ml > self.chamber_l * 750.0
            || self.source_ml > self.receiver_capacity_ml
            || self.pumping_l_s > 10.0
            || self.leak_l_s > 10.0
            || self.filter_ml_s_kpa > 1.0
        {
            return Err("invalid filtration parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Assembly {
    Detached,
    Placed,
    Sealed,
}
#[derive(Debug, Clone)]
pub struct FiltrationState {
    pub config: FiltrationConfig,
    assembly: Assembly,
    filter: bool,
    slurry: bool,
    vent: bool,
    running: bool,
    pressure: f64,
    source_ml: f64,
    receiver_ml: f64,
    removed_ml: f64,
    introduced_ml: f64,
}
impl FiltrationState {
    pub fn new(config: FiltrationConfig) -> Self {
        Self {
            pressure: config.ambient_kpa,
            config,
            assembly: Assembly::Detached,
            filter: false,
            slurry: false,
            vent: false,
            running: false,
            source_ml: 0.0,
            receiver_ml: 0.0,
            removed_ml: 0.0,
            introduced_ml: 0.0,
        }
    }
    fn require_vented(&self) -> Result<(), String> {
        if self.running || self.config.ambient_kpa - self.pressure > 0.5 {
            Err("VENT_FIRST".into())
        } else {
            Ok(())
        }
    }
    pub fn apply(
        &mut self,
        device: &str,
        action: &str,
        params: &Value,
        samples: &mut SampleInventory,
    ) -> Result<(), String> {
        thermal::empty(params)?;
        match action {
            "assemble" => {
                self.require_vented()?;
                if self.assembly == Assembly::Sealed {
                    return Err("UNSEAL_FIRST".into());
                }
                if self.slurry {
                    return Err("CLEAR_SLURRY_FIRST".into());
                }
                self.assembly = if self.assembly == Assembly::Detached {
                    Assembly::Placed
                } else {
                    Assembly::Detached
                };
            }
            "seal" => {
                self.require_vented()?;
                self.assembly = match self.assembly {
                    Assembly::Detached => return Err("ASSEMBLE_FIRST".into()),
                    Assembly::Placed => Assembly::Sealed,
                    Assembly::Sealed => Assembly::Placed,
                };
            }
            "install_filter" | "remove_filter" => {
                self.require_vented()?;
                if self.assembly != Assembly::Placed {
                    return Err("OPEN_FUNNEL_FIRST".into());
                }
                if self.slurry {
                    return Err("CLEAR_SLURRY_FIRST".into());
                }
                if (action == "install_filter") == self.filter {
                    return Err("FILTER_STATE_UNCHANGED".into());
                }
                self.filter = action == "install_filter";
            }
            "load_slurry" => {
                self.require_vented()?;
                if self.assembly != Assembly::Placed {
                    return Err("OPEN_FUNNEL_FIRST".into());
                }
                if !self.filter {
                    return Err("INSTALL_FILTER_FIRST".into());
                }
                if self.receiver_ml + self.config.source_ml > self.config.receiver_capacity_ml {
                    return Err("EMPTY_RECEIVER_FIRST".into());
                }
                samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                self.slurry = true;
                self.source_ml = self.config.source_ml;
                self.introduced_ml += self.config.source_ml;
            }
            "clear_slurry" => {
                self.require_vented()?;
                if self.assembly != Assembly::Placed {
                    return Err("OPEN_FUNNEL_FIRST".into());
                }
                samples.release(&self.config.sample_id, device)?;
                self.slurry = false;
                self.removed_ml += self.source_ml;
                self.source_ml = 0.0;
            }
            "empty_receiver" => {
                self.require_vented()?;
                if self.assembly != Assembly::Detached {
                    return Err("REMOVE_FUNNEL_FIRST".into());
                }
                self.removed_ml += self.receiver_ml;
                self.receiver_ml = 0.0;
            }
            "start" => {
                if self.assembly != Assembly::Sealed {
                    return Err("SEAL_FIRST".into());
                }
                if !self.filter {
                    return Err("INSTALL_FILTER_FIRST".into());
                }
                if self.source_ml <= 0.0 {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
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
            "reset" => {
                self.require_vented()?;
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    fn step(&mut self, h: f64) {
        let c = &self.config;
        let volume = c.chamber_l - self.receiver_ml / 1000.0;
        let leak = if self.vent || self.assembly != Assembly::Sealed {
            2.0
        } else {
            c.leak_l_s
        };
        let next = pressure_after(
            self.pressure,
            h,
            if self.running { c.pumping_l_s } else { 0.0 },
            leak,
            c.ultimate_kpa,
            c.ambient_kpa,
            volume,
        );
        let transfer = if self.filter && self.assembly == Assembly::Sealed {
            (c.filter_ml_s_kpa * (c.ambient_kpa - (self.pressure + next) * 0.5).max(0.0) * h)
                .min(self.source_ml)
                .min(c.receiver_capacity_ml - self.receiver_ml)
        } else {
            0.0
        };
        self.source_ml -= transfer;
        self.receiver_ml += transfer;
        self.pressure = next * volume / (volume - transfer / 1000.0);
        if self.source_ml <= 0.0 || self.receiver_ml >= c.receiver_capacity_ml {
            self.running = false;
        }
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let before = (self.pressure, self.source_ml, self.running);
        let steps = (dt / 0.02).ceil().max(1.0) as u32;
        for _ in 0..steps {
            self.step(dt / steps as f64);
        }
        before != (self.pressure, self.source_ml, self.running)
    }
    pub fn values(&self) -> Value {
        json!({
            "pressure_kpa":self.pressure,
            "source_ml":self.source_ml,
            "receiver_ml":self.receiver_ml,
            "removed_ml":self.removed_ml,
            "introduced_ml":self.introduced_ml,
            "source_fraction":self.source_ml/self.config.source_ml,
            "receiver_fraction":self.receiver_ml/self.config.receiver_capacity_ml,
            "source_liquid_visible":self.source_ml>0.0,
            "receiver_liquid_visible":self.receiver_ml>0.0,
            "funnel_visible":self.assembly!=Assembly::Detached,
            "filter_visible":self.filter,
            "sealed":self.assembly==Assembly::Sealed,
            "seal_rad":if self.assembly==Assembly::Sealed{0.0}else{std::f64::consts::PI / 3.0},
            "vent_rad":if self.vent{std::f64::consts::FRAC_PI_2}else{0.0},
            "running":self.running,
            "status":if self.vent{"venting"}else if self.running{"filtering"}else if self.config.ambient_kpa-self.pressure>0.5{"vacuum"}else{"idle"}
        })
    }
}
