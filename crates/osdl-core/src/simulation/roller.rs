//! Original parallel roller fixture with declared no-slip kinematics, not contact solving.
use super::{motor::Motor, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::f64::consts::TAU;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollerConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub roller_radius_m: f64,
    pub tube_radius_m: f64,
    pub sample_id: String,
}
impl RollerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.max_rpm,
                self.slew_rpm_s,
                self.roller_radius_m,
                self.tube_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("invalid tube roller parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct RollerState {
    pub config: RollerConfig,
    motor: Motor,
    tube_angle: f64,
    loaded: bool,
}
impl RollerState {
    pub fn new(config: RollerConfig) -> Self {
        Self {
            motor: Motor::new(config.max_rpm, config.slew_rpm_s),
            config,
            tube_angle: 0.0,
            loaded: false,
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
            "load_tube" | "unload_tube" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                if action == "load_tube" {
                    samples.claim(&self.config.sample_id, device, self.config.tube_radius_m)?;
                    self.loaded = true;
                } else {
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                }
            }
            "start" => {
                thermal::empty(params)?;
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
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
        let radians = self.motor.advance(dt);
        if self.loaded {
            self.tube_angle = (self.tube_angle
                - radians * self.config.roller_radius_m / self.config.tube_radius_m)
                .rem_euclid(TAU);
        }
        radians > 0.0
    }
    pub fn motion(&self) -> Value {
        json!({"roller_rad_s": self.motor.motion(1.0, None), "tube_rad_s": self.motor.motion(-self.config.roller_radius_m/self.config.tube_radius_m, None)})
    }
    pub fn values(&self) -> Value {
        json!({"rpm":self.motor.rpm, "target_rpm":self.motor.target,
            "roller_rad":self.motor.angle, "roller_rad_s":self.motor.velocity(),
            "tube_rad":self.tube_angle, "tube_rad_s":if self.loaded { -self.motor.velocity()*self.config.roller_radius_m/self.config.tube_radius_m } else {0.0},
            "speed_dial_rad":self.motor.dial(), "tube_visible":self.loaded,
            "running":self.motor.running, "status":self.motor.status()})
    }
}
