//! Original level-platform orbital shaker. Prescribed motion, not fluid/contact dynamics.
use super::{motor::Motor, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitalConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub orbit_m: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl OrbitalConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.max_rpm,
                self.slew_rpm_s,
                self.orbit_m,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("invalid orbital shaker parameters".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct OrbitalState {
    pub config: OrbitalConfig,
    motor: Motor,
    loaded: bool,
    secured: bool,
}
impl OrbitalState {
    pub fn new(config: OrbitalConfig) -> Self {
        Self {
            motor: Motor::new(config.max_rpm, config.slew_rpm_s),
            config,
            loaded: false,
            secured: false,
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
            "set_speed" => self.motor.set_speed(params)?,
            "load_flask" | "unload_flask" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                if self.secured {
                    return Err("UNCLAMP_SAMPLE_FIRST".into());
                }
                if action == "load_flask" {
                    samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                    self.loaded = true;
                } else {
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                }
            }
            "clamp" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.secured = !self.secured;
            }
            "start" => {
                thermal::empty(params)?;
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                if !self.secured {
                    return Err("SECURE_SAMPLE_FIRST".into());
                }
                self.motor.start()?;
            }
            "stop" => {
                thermal::empty(params)?;
                self.motor.running = false;
            }
            "reset" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }

    pub fn advance(&mut self, dt: f64) -> bool {
        self.motor.advance(dt) > 0.0
    }

    pub fn values(&self) -> Value {
        let velocity = self.motor.velocity();
        json!({
            "rpm": self.motor.rpm, "target_rpm": self.motor.target,
            "orbit_rad": self.motor.angle, "orbit_rad_s": velocity,
            "platform_rad": -self.motor.angle, "platform_rad_s": -velocity,
            "speed_dial_rad": self.motor.dial(),
            "clamp_rad": if self.secured { 0.0 } else { 0.35 },
            "flask_visible": self.loaded, "secured": self.secured, "running": self.motor.running,
            "status": self.motor.status()
        })
    }
}
