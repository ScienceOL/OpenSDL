//! Current-state motor kinematics shared by the original bench instruments.
use super::thermal;
use serde_json::Value;
use std::f64::consts::TAU;

/// A bounded prediction from the current observation, in radians and seconds.
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct Motion {
    target_velocity: f64,
    slew_per_s: f64,
    stop_after_s: Option<f64>,
}

pub(super) fn profile(
    target_rpm: f64,
    slew_rpm_s: f64,
    ratio: f64,
    stop_after_s: Option<f64>,
) -> Motion {
    Motion {
        target_velocity: target_rpm * TAU / 60.0 * ratio,
        slew_per_s: slew_rpm_s * TAU / 60.0 * ratio.abs(),
        stop_after_s,
    }
}

/// Exact ramp area; shared by every motor's live advancement and prediction data.
pub(super) fn advance_rpm(before: f64, target: f64, slew: f64, dt: f64) -> (f64, f64) {
    let next = thermal::approach(before, target, slew, dt);
    let ramp_s = ((target - before).abs() / slew).min(dt);
    let turns = ((before + next) * 0.5 * ramp_s + next * (dt - ramp_s)) / 60.0;
    (next, turns * TAU)
}

#[derive(Debug, Clone)]
pub(super) struct Motor {
    max_rpm: f64,
    slew_rpm_s: f64,
    pub target: f64,
    pub rpm: f64,
    pub angle: f64,
    pub running: bool,
}
impl Motor {
    pub fn new(max_rpm: f64, slew_rpm_s: f64) -> Self {
        Self {
            max_rpm,
            slew_rpm_s,
            target: 0.0,
            rpm: 0.0,
            angle: 0.0,
            running: false,
        }
    }
    pub fn set_speed(&mut self, params: &Value) -> Result<(), String> {
        self.target = thermal::number(params, "rpm", 0.0, self.max_rpm)?;
        Ok(())
    }
    pub fn require_stopped(&self) -> Result<(), String> {
        if self.running || self.rpm > 0.0 {
            Err("MOTOR_MOVING".into())
        } else {
            Ok(())
        }
    }
    pub fn start(&mut self) -> Result<(), String> {
        if self.target <= 0.0 {
            return Err("SET_SPEED_FIRST".into());
        }
        self.running = true;
        Ok(())
    }
    /// Return the travelled angle so geared parts integrate without wrap discontinuities.
    pub fn advance(&mut self, dt: f64) -> f64 {
        let target = if self.running { self.target } else { 0.0 };
        let (rpm, radians) = advance_rpm(self.rpm, target, self.slew_rpm_s, dt);
        self.rpm = rpm;
        self.angle = (self.angle + radians).rem_euclid(TAU);
        radians
    }
    /// Inverse of the same ramp integral, for an attainable angle within a step.
    pub fn time_to_angle(&self, radians: f64) -> f64 {
        if radians <= 0.0 {
            return 0.0;
        }
        let target = if self.running { self.target } else { 0.0 };
        let ramp = (target - self.rpm).abs() / self.slew_rpm_s;
        let ramp_area = (self.rpm + target) * 0.5 * ramp;
        let area = radians * 60.0 / TAU;
        if ramp > 0.0 && area <= ramp_area {
            let acceleration = (target - self.rpm).signum() * self.slew_rpm_s;
            let speed = (self.rpm * self.rpm + 2.0 * acceleration * area)
                .max(0.0)
                .sqrt();
            2.0 * area / (self.rpm + speed)
        } else if target > 0.0 {
            ramp + (area - ramp_area) / target
        } else {
            ramp
        }
    }
    pub fn velocity(&self) -> f64 {
        self.rpm * TAU / 60.0
    }
    pub fn motion(&self, ratio: f64, stop_after_s: Option<f64>) -> Motion {
        profile(
            if self.running { self.target } else { 0.0 },
            self.slew_rpm_s,
            ratio,
            stop_after_s,
        )
    }
    pub fn dial(&self) -> f64 {
        self.target / self.max_rpm * TAU * 0.75
    }
    pub fn status(&self) -> &'static str {
        if self.running {
            "running"
        } else if self.rpm > 0.0 {
            "decelerating"
        } else {
            "idle"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ramps_match_the_shared_client_trajectory_vectors() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../web/instrument-model/motion-fixtures.json"
        ))
        .unwrap();
        for f in fixtures.as_array().unwrap() {
            let mut motor = Motor::new(1000.0, f["slew_rpm_s"].as_f64().unwrap());
            motor.rpm = f["rpm"].as_f64().unwrap();
            motor.target = f["target_rpm"].as_f64().unwrap();
            motor.running = true;
            let dt = f["seconds"].as_f64().unwrap();
            let radians = match f["stop_after_s"].as_f64() {
                Some(stop) => {
                    let first = motor.advance(stop);
                    motor.running = false;
                    first + motor.advance(dt - stop)
                }
                None => motor.advance(dt),
            } * f["ratio"].as_f64().unwrap();
            assert!(
                (radians - f["expected_radians"].as_f64().unwrap()).abs() < 1e-10,
                "{}",
                f["name"]
            );
        }
    }
}
