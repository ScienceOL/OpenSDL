//! Original height-adjustable overhead stirrer with explicit stand clamp and vessel envelope.
use super::{motor::Motor, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverheadConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub lift_m: f64,
    pub immersion_max_m: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl OverheadConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.max_rpm,
                self.slew_rpm_s,
                self.lift_m,
                self.immersion_max_m,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.immersion_max_m >= self.lift_m
        {
            return Err("invalid overhead stirrer parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct OverheadState {
    pub config: OverheadConfig,
    motor: Motor,
    height_m: f64,
    secured: bool,
    loaded: bool,
}
impl OverheadState {
    pub fn new(config: OverheadConfig) -> Self {
        Self {
            motor: Motor::new(config.max_rpm, config.slew_rpm_s),
            height_m: config.lift_m,
            config,
            secured: false,
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
            "load_vessel" | "unload_vessel" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                if self.height_m < self.config.lift_m {
                    return Err("RAISE_SHAFT_FIRST".into());
                }
                if action == "load_vessel" {
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
                self.secured = !self.secured;
            }
            "set_height" | "lift" => {
                let height = if action == "lift" {
                    thermal::empty(params)?;
                    if self.height_m > 0.0 {
                        0.0
                    } else {
                        self.config.lift_m
                    }
                } else {
                    thermal::number(params, "height_m", 0.0, self.config.lift_m)?
                };
                self.motor.require_stopped()?;
                if self.secured {
                    return Err("UNCLAMP_STAND_FIRST".into());
                }
                self.height_m = height;
            }
            "start" => {
                thermal::empty(params)?;
                if !self.loaded {
                    return Err("LOAD_VESSEL_FIRST".into());
                }
                if !self.secured {
                    return Err("SECURE_STAND_FIRST".into());
                }
                if self.height_m > self.config.immersion_max_m {
                    return Err("IMMERSE_SHAFT_FIRST".into());
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
    pub fn motion(&self) -> Value {
        json!({"shaft_rad_s": self.motor.motion(1.0, None)})
    }
    pub fn values(&self) -> Value {
        json!({"rpm":self.motor.rpm,"target_rpm":self.motor.target,"shaft_rad":self.motor.angle,
            "shaft_rad_s":self.motor.velocity(),"speed_dial_rad":self.motor.dial(),"carriage_m":self.height_m,
            "clamp_rad":if self.secured {0.0}else{0.6},"secured":self.secured,
            "vessel_visible":self.loaded,"running":self.motor.running,"status":self.motor.status()})
    }
}
