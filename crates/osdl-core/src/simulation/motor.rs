//! Current-state motor kinematics shared by the original bench instruments.
use super::thermal;
use serde_json::Value;
use std::f64::consts::TAU;

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
        let before = self.rpm;
        self.rpm = thermal::approach(before, target, self.slew_rpm_s, dt);
        let ramp_s = ((target - before).abs() / self.slew_rpm_s).min(dt);
        let turns = ((before + self.rpm) * 0.5 * ramp_s + self.rpm * (dt - ramp_s)) / 60.0;
        let radians = turns * TAU;
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
