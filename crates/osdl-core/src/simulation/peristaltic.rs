//! Original peristaltic pump with declared displacement and explicit liquid inventories.
use super::{motor::Motor, thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::f64::consts::TAU;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeristalticConfig {
    pub model_id: String,
    pub max_rpm: f64,
    pub slew_rpm_s: f64,
    pub displacement_ml_rev: f64,
    pub roller_orbit_m: f64,
    pub roller_radius_m: f64,
    pub source_ml: f64,
    pub receiver_capacity_ml: f64,
    pub dead_volume_ml: f64,
    pub opening_rad: f64,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl PeristalticConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || [
                self.max_rpm,
                self.slew_rpm_s,
                self.displacement_ml_rev,
                self.roller_orbit_m,
                self.roller_radius_m,
                self.source_ml,
                self.receiver_capacity_ml,
                self.dead_volume_ml,
                self.opening_rad,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || !(self.max_rpm * self.displacement_ml_rev).is_finite()
            || !(self.roller_orbit_m / self.roller_radius_m).is_finite()
            || self.dead_volume_ml >= self.source_ml
        {
            return Err("invalid peristaltic pump parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct PeristalticState {
    pub config: PeristalticConfig,
    motor: Motor,
    roller_angle: f64,
    opened: bool,
    loaded: bool,
    source_ml: f64,
    receiver_ml: f64,
    line_ml: f64,
    removed_ml: f64,
}
impl PeristalticState {
    pub fn new(config: PeristalticConfig) -> Self {
        Self {
            motor: Motor::new(config.max_rpm, config.slew_rpm_s),
            roller_angle: 0.0,
            source_ml: config.source_ml,
            config,
            opened: false,
            loaded: false,
            receiver_ml: 0.0,
            line_ml: 0.0,
            removed_ml: 0.0,
        }
    }
    fn require_connected(&self) -> Result<(), String> {
        if self.opened {
            return Err("CLOSE_LID_FIRST".into());
        }
        if !self.loaded {
            return Err("LOAD_TUBING_FIRST".into());
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
            "set_flow" => {
                let flow = thermal::number(
                    params,
                    "flow_ml_min",
                    0.0,
                    self.config.max_rpm * self.config.displacement_ml_rev,
                )?;
                self.motor
                    .set_speed(&json!({"rpm":flow/self.config.displacement_ml_rev}))?;
            }
            "lid" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                self.opened = !self.opened;
            }
            "load_tubing" | "unload_tubing" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                if action == "load_tubing" {
                    samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                    self.loaded = true;
                } else {
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                    self.removed_ml += self.line_ml;
                    self.line_ml = 0.0;
                }
            }
            "prime" => {
                thermal::empty(params)?;
                self.motor.require_stopped()?;
                self.require_connected()?;
                let required = self.config.dead_volume_ml - self.line_ml;
                if self.source_ml < required {
                    return Err("INSUFFICIENT_VOLUME".into());
                }
                self.source_ml -= required;
                self.line_ml = self.config.dead_volume_ml;
            }
            "start" => {
                thermal::empty(params)?;
                self.require_connected()?;
                if self.line_ml < self.config.dead_volume_ml {
                    return Err("PRIME_FIRST".into());
                }
                if self.source_ml <= 0.0 {
                    return Err("SOURCE_EMPTY".into());
                }
                if self.receiver_ml >= self.config.receiver_capacity_ml {
                    return Err("RECEIVER_FULL".into());
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
        let connected = self.loaded && self.line_ml >= self.config.dead_volume_ml;
        let available = self
            .source_ml
            .min(self.config.receiver_capacity_ml - self.receiver_ml);
        if connected && available <= 0.0 {
            self.motor.running = false;
        }
        let before = self.motor.clone();
        let mut radians = self.motor.advance(dt);
        if connected {
            let requested = radians / TAU * self.config.displacement_ml_rev;
            let volume = if available > 0.0 && requested >= available {
                // Stop at the actual capacity boundary, then coast for the rest of the step.
                let until_limit = before
                    .time_to_angle(available / self.config.displacement_ml_rev * TAU)
                    .clamp(0.0, dt);
                self.motor = before;
                radians = self.motor.advance(until_limit);
                self.motor.running = false;
                radians += self.motor.advance(dt - until_limit);
                available
            } else {
                requested.min(available)
            };
            self.source_ml -= volume;
            self.receiver_ml += volume;
            if self.source_ml <= 0.0 || self.receiver_ml >= self.config.receiver_capacity_ml {
                self.motor.running = false;
            }
        }
        let ratio = 1.0 + self.config.roller_orbit_m / self.config.roller_radius_m;
        self.roller_angle = (self.roller_angle - ratio * radians).rem_euclid(TAU);
        radians > 0.0
    }
    pub fn motion(&self) -> Value {
        let available = self
            .source_ml
            .min(self.config.receiver_capacity_ml - self.receiver_ml)
            .max(0.0);
        let stop = (self.motor.running
            && self.loaded
            && self.line_ml >= self.config.dead_volume_ml)
            .then(|| {
                self.motor
                    .time_to_angle(available / self.config.displacement_ml_rev * TAU)
            });
        let ratio = -(1.0 + self.config.roller_orbit_m / self.config.roller_radius_m);
        json!({"rotor_rad_s": self.motor.motion(1.0, stop), "roller_rad_s": self.motor.motion(ratio, stop)})
    }
    pub fn values(&self) -> Value {
        let primed = self.loaded && self.line_ml >= self.config.dead_volume_ml;
        let roller_ratio = 1.0 + self.config.roller_orbit_m / self.config.roller_radius_m;
        let flowing =
            primed && self.source_ml > 0.0 && self.receiver_ml < self.config.receiver_capacity_ml;
        let status = if self.opened {
            "open"
        } else if !self.loaded {
            "no_tube"
        } else if !primed {
            "unprimed"
        } else if !self.motor.running && self.motor.rpm > 0.0 {
            "decelerating"
        } else if self.source_ml <= 0.0 {
            "empty"
        } else if self.receiver_ml >= self.config.receiver_capacity_ml {
            "full"
        } else if self.motor.running {
            "pumping"
        } else {
            "idle"
        };
        json!({
            "rpm": self.motor.rpm,
            "target_rpm": self.motor.target,
            "rotor_rad": self.motor.angle,
            "rotor_rad_s": self.motor.velocity(),
            "roller_rad": self.roller_angle,
            "roller_rad_s": -self.motor.velocity()*roller_ratio,
            "flow_ml_min": if flowing {self.motor.rpm*self.config.displacement_ml_rev}else{0.0},
            "target_flow_ml_min": self.motor.target*self.config.displacement_ml_rev,
            "flow_dial_rad": self.motor.dial(),
            "source_ml": self.source_ml,
            "receiver_ml": self.receiver_ml,
            "line_ml": self.line_ml,
            "removed_ml": self.removed_ml,
            "source_fraction": self.source_ml/self.config.source_ml,
            "receiver_fraction": self.receiver_ml/self.config.receiver_capacity_ml,
            "source_liquid_visible": self.source_ml>0.0,
            "receiver_liquid_visible": self.receiver_ml>0.0,
            "tube_visible": self.loaded,
            "liquid_visible": primed,
            "primed": primed,
            "lid_rad": if self.opened {self.config.opening_rad}else{0.0},
            "running": self.motor.running,
            "status": status
        })
    }
}
