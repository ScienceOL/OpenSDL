//! Original eccentric cup oscillator. Pose only, not fluid mixing efficiency.
use super::{motor, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VortexConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub orbit_m: f64,
}
impl VortexConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || [self.max_rpm, self.slew_rpm_s, self.orbit_m]
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("invalid vortex parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct VortexState {
    pub config: VortexConfig,
    target: f64,
    rpm: f64,
    angle: f64,
    loaded: bool,
    running: bool,
}
impl VortexState {
    pub fn new(config: VortexConfig) -> Self {
        Self {
            config,
            target: 0.0,
            rpm: 0.0,
            angle: 0.0,
            loaded: false,
            running: false,
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
            "load_tube" => {
                thermal::empty(params)?;
                if self.rpm > 0.0 {
                    return Err("MOTOR_MOVING".into());
                }
                samples.claim("vortex_tube", device, 0.02)?;
                self.loaded = true;
            }
            "unload_tube" => {
                thermal::empty(params)?;
                if self.rpm > 0.0 || self.running {
                    return Err("MOTOR_MOVING".into());
                }
                samples.release("vortex_tube", device)?;
                self.loaded = false;
            }
            "start" => {
                thermal::empty(params)?;
                if self.target <= 0.0 {
                    return Err("SET_SPEED_FIRST".into());
                }
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.running = true;
            }
            "stop" => {
                thermal::empty(params)?;
                self.running = false;
            }
            "reset" => {
                thermal::empty(params)?;
                if self.rpm > 0.0 {
                    return Err("MOTOR_MOVING".into());
                }
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let old = (self.rpm, self.angle);
        let (rpm, radians) = motor::advance_rpm(
            self.rpm,
            if self.running { self.target } else { 0.0 },
            self.config.slew_rpm_s,
            dt,
        );
        self.rpm = rpm;
        self.angle = (self.angle + radians).rem_euclid(std::f64::consts::TAU);
        // Repeated sampled angles still represent motion (e.g. one turn per tick).
        self.rpm > 0.0 || old != (self.rpm, self.angle)
    }
    pub fn motion(&self) -> Value {
        let target = if self.running { self.target } else { 0.0 };
        json!({"orbit_rad_s": motor::profile(target, self.config.slew_rpm_s, 1.0, None), "cup_rad_s": motor::profile(target, self.config.slew_rpm_s, -1.0, None)})
    }
    pub fn values(&self) -> Value {
        json!({
            "rpm":self.rpm,
            "target_rpm":self.target,
            "orbit_rad":self.angle,
            "cup_rad":-self.angle,
            "cup_rad_s":-self.rpm*std::f64::consts::TAU/60.0,
            "orbit_rad_s":self.rpm*std::f64::consts::TAU/60.0,
            "speed_dial_rad":self.target/self.config.max_rpm*std::f64::consts::TAU*0.75,
            "tube_visible":self.loaded,
            "running":self.running,
            "status":if self.running {"running"}else if self.rpm>0.0 {"decelerating"}else{"idle"}})
    }
}
