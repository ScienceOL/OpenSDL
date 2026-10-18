//! Original level-platform orbital shaker. Prescribed motion, not fluid/contact dynamics.
use super::{thermal, SampleInventory};
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
    target: f64,
    rpm: f64,
    angle: f64,
    loaded: bool,
    secured: bool,
    running: bool,
}
impl OrbitalState {
    pub fn new(config: OrbitalConfig) -> Self {
        Self {
            config,
            target: 0.0,
            rpm: 0.0,
            angle: 0.0,
            loaded: false,
            secured: false,
            running: false,
        }
    }

    fn require_stopped(&self) -> Result<(), String> {
        if self.running || self.rpm > 0.0 {
            Err("MOTOR_MOVING".into())
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
        match action {
            "set_speed" => self.target = thermal::number(params, "rpm", 0.0, self.config.max_rpm)?,
            "load_flask" | "unload_flask" => {
                thermal::empty(params)?;
                self.require_stopped()?;
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
                self.require_stopped()?;
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.secured = !self.secured;
            }
            "start" => {
                thermal::empty(params)?;
                if self.target <= 0.0 {
                    return Err("SET_SPEED_FIRST".into());
                }
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                if !self.secured {
                    return Err("SECURE_SAMPLE_FIRST".into());
                }
                self.running = true;
            }
            "stop" => {
                thermal::empty(params)?;
                self.running = false;
            }
            "reset" => {
                thermal::empty(params)?;
                self.require_stopped()?;
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }

    pub fn advance(&mut self, dt: f64) -> bool {
        let target = if self.running { self.target } else { 0.0 };
        let before = self.rpm;
        self.rpm = thermal::approach(before, target, self.config.slew_rpm_s, dt);
        // Integrate the ramp and any remaining constant-speed segment exactly.
        let ramp_s = (target - before).abs() / self.config.slew_rpm_s;
        let ramp_s = ramp_s.min(dt);
        let turns = ((before + self.rpm) * 0.5 * ramp_s + self.rpm * (dt - ramp_s)) / 60.0;
        self.angle = (self.angle + turns * std::f64::consts::TAU).rem_euclid(std::f64::consts::TAU);
        before > 0.0 || self.rpm > 0.0
    }

    pub fn values(&self) -> Value {
        let velocity = self.rpm * std::f64::consts::TAU / 60.0;
        json!({
            "rpm": self.rpm, "target_rpm": self.target,
            "orbit_rad": self.angle, "orbit_rad_s": velocity,
            "platform_rad": -self.angle, "platform_rad_s": -velocity,
            "speed_dial_rad": self.target / self.config.max_rpm * std::f64::consts::TAU * 0.75,
            "clamp_rad": if self.secured { 0.0 } else { 0.35 },
            "flask_visible": self.loaded, "secured": self.secured, "running": self.running,
            "status": if self.running { "running" } else if self.rpm > 0.0 { "decelerating" } else { "idle" }
        })
    }
}
