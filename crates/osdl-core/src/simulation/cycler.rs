//! Bounded demonstration temperature schedule; no biological amplification model.
use super::{
    thermal::{self, ThermalPair},
    SampleInventory,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalStage {
    pub temperature_c: f64,
    pub hold_s: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CyclerConfig {
    pub model_id: String,
    pub ambient_c: f64,
    pub heater_w: f64,
    pub cooling_w: f64,
    pub block_capacity_j_k: f64,
    pub sample_capacity_j_k: f64,
    pub exchange_w_k: f64,
    pub loss_w_k: f64,
    pub lid_heater_w: f64,
    pub lid_capacity_j_k: f64,
    pub lid_target_c: f64,
    pub opening_rad: f64,
    pub max_cycles: u32,
    pub stages: Vec<ThermalStage>,
    pub sample_id: String,
    pub sample_radius_m: f64,
}
impl CyclerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.sample_id.is_empty()
            || !(0.0..40.0).contains(&self.ambient_c)
            || [
                self.ambient_c,
                self.heater_w,
                self.cooling_w,
                self.block_capacity_j_k,
                self.sample_capacity_j_k,
                self.exchange_w_k,
                self.loss_w_k,
                self.lid_heater_w,
                self.lid_capacity_j_k,
                self.lid_target_c,
                self.opening_rad,
                self.sample_radius_m,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.heater_w > 1000.0
            || self.cooling_w > 1000.0
            || self.lid_heater_w > 200.0
            || self.lid_target_c > 110.0
            || self.lid_target_c < 95.0
            || self.max_cycles == 0
            || self.max_cycles > 40
            || self.stages.is_empty()
            || self.stages.len() > 8
            || self.stages.iter().any(|s| {
                !s.temperature_c.is_finite()
                    || !(40.0..=99.0).contains(&s.temperature_c)
                    || !s.hold_s.is_finite()
                    || !(1.0..=3600.0).contains(&s.hold_s)
            })
            || (self.exchange_w_k + self.loss_w_k * 5.0) / self.block_capacity_j_k > 10.0
            || (self.exchange_w_k + 0.1) / self.sample_capacity_j_k > 10.0
            || 1.5 / self.lid_capacity_j_k > 10.0
        {
            return Err("invalid thermal demonstration program".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Ramping,
    Holding,
    Cooling { aborted: bool },
    Complete { aborted: bool },
}
#[derive(Debug, Clone)]
pub struct CyclerState {
    pub config: CyclerConfig,
    thermal: ThermalPair,
    lid_c: f64,
    opened: bool,
    latched: bool,
    loaded: bool,
    phase: Phase,
    cycles: u32,
    cycle: u32,
    stage: usize,
    hold_remaining: f64,
}
impl CyclerState {
    pub fn new(config: CyclerConfig) -> Self {
        Self {
            thermal: ThermalPair::new(config.ambient_c),
            lid_c: config.ambient_c,
            cycles: config.max_cycles.min(3),
            config,
            opened: false,
            latched: false,
            loaded: false,
            phase: Phase::Idle,
            cycle: 0,
            stage: 0,
            hold_remaining: 0.0,
        }
    }
    fn active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Ramping | Phase::Holding | Phase::Cooling { .. }
        )
    }
    fn hot(&self) -> bool {
        self.thermal.plate_c > 45.0 || self.thermal.liquid_c > 45.0 || self.lid_c > 45.0
    }
    fn require_cool_stopped(&self) -> Result<(), String> {
        if self.active() || self.hot() {
            Err("WAIT_FOR_COOL_STOP".into())
        } else {
            Ok(())
        }
    }
    fn target(&self) -> f64 {
        if matches!(self.phase, Phase::Ramping | Phase::Holding) {
            self.config.stages[self.stage].temperature_c
        } else {
            self.config.ambient_c
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
            "set_cycles" => {
                let cycles = thermal::number(params, "cycles", 1.0, self.config.max_cycles as f64)?;
                if cycles.fract() != 0.0 {
                    return Err("INVALID_PARAMS".into());
                }
                self.require_cool_stopped()?;
                self.cycles = cycles as u32;
            }
            "lid" => {
                thermal::empty(params)?;
                self.require_cool_stopped()?;
                if self.latched {
                    return Err("UNLATCH_FIRST".into());
                }
                self.opened = !self.opened;
            }
            "latch" => {
                thermal::empty(params)?;
                self.require_cool_stopped()?;
                if self.opened {
                    return Err("CLOSE_LID_FIRST".into());
                }
                self.latched = !self.latched;
            }
            "load_tubes" | "unload_tubes" => {
                thermal::empty(params)?;
                self.require_cool_stopped()?;
                if !self.opened {
                    return Err("OPEN_DOOR_FIRST".into());
                }
                if action == "load_tubes" {
                    samples.claim(&self.config.sample_id, device, self.config.sample_radius_m)?;
                    self.loaded = true;
                    self.thermal.liquid_c = self.config.ambient_c;
                } else {
                    samples.release(&self.config.sample_id, device)?;
                    self.loaded = false;
                }
            }
            "start" => {
                thermal::empty(params)?;
                self.require_cool_stopped()?;
                if self.opened {
                    return Err("CLOSE_LID_FIRST".into());
                }
                if !self.latched {
                    return Err("LATCH_FIRST".into());
                }
                if !self.loaded {
                    return Err("LOAD_SAMPLE_FIRST".into());
                }
                self.phase = Phase::Ramping;
                self.cycle = 1;
                self.stage = 0;
                self.hold_remaining = self.config.stages[0].hold_s;
            }
            "stop" => {
                thermal::empty(params)?;
                if self.active() || self.hot() {
                    self.phase = Phase::Cooling { aborted: true };
                }
            }
            "reset" => {
                thermal::empty(params)?;
                self.require_cool_stopped()?;
                if self.thermal.plate_c > self.config.ambient_c + 2.0
                    || (self.loaded && self.thermal.liquid_c > self.config.ambient_c + 2.0)
                    || self.lid_c > self.config.ambient_c + 2.0
                {
                    return Err("WAIT_FOR_AMBIENT_STOP".into());
                }
                samples.release_device(device);
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    fn step(&mut self, h: f64) {
        let c = &self.config;
        let program = matches!(self.phase, Phase::Ramping | Phase::Holding);
        let enabled = self.active();
        let target = self.target();
        self.thermal.advance(
            h,
            |temperature| {
                if enabled {
                    ((target - temperature) * 100.0).clamp(-c.cooling_w, c.heater_w)
                } else {
                    0.0
                }
            },
            [c.block_capacity_j_k, c.sample_capacity_j_k],
            [
                c.exchange_w_k,
                c.loss_w_k * if self.opened { 5.0 } else { 1.0 },
                0.1,
            ],
            c.ambient_c,
            self.loaded,
        );
        let lid_power = if program {
            ((c.lid_target_c - self.lid_c) * 10.0).clamp(0.0, c.lid_heater_w)
        } else {
            0.0
        };
        self.lid_c +=
            (lid_power - (self.lid_c - c.ambient_c) * if self.opened { 1.5 } else { 0.3 }) * h
                / c.lid_capacity_j_k;
        if program {
            if (self.thermal.liquid_c - target).abs() <= 1.0 && self.lid_c >= c.lid_target_c - 10.0
            {
                self.phase = Phase::Holding;
                self.hold_remaining = (self.hold_remaining - h).max(0.0);
            } else {
                self.phase = Phase::Ramping;
            }
            if self.hold_remaining == 0.0 {
                if self.stage + 1 < c.stages.len() {
                    self.stage += 1;
                } else if self.cycle < self.cycles {
                    self.stage = 0;
                    self.cycle += 1;
                } else {
                    self.phase = Phase::Cooling { aborted: false };
                    return;
                }
                self.hold_remaining = c.stages[self.stage].hold_s;
                self.phase = Phase::Ramping;
            }
        } else if let Phase::Cooling { aborted } = self.phase {
            if self.thermal.plate_c <= c.ambient_c + 1.0
                && self.thermal.liquid_c <= c.ambient_c + 2.0
                && self.lid_c <= 45.0
            {
                self.phase = Phase::Complete { aborted };
            }
        }
    }

    pub fn advance(&mut self, dt: f64) -> bool {
        let before = (
            self.thermal.plate_c,
            self.thermal.liquid_c,
            self.lid_c,
            self.phase,
            self.hold_remaining,
        );
        let count = (dt / 0.02).ceil().max(1.0) as u32;
        for _ in 0..count {
            self.step(dt / count as f64);
        }
        before
            != (
                self.thermal.plate_c,
                self.thermal.liquid_c,
                self.lid_c,
                self.phase,
                self.hold_remaining,
            )
    }
    pub fn values(&self) -> Value {
        json!({
            "temperature_c":self.thermal.plate_c,
            "sample_c":self.loaded.then_some(self.thermal.liquid_c),
            "lid_temperature_c":self.lid_c,
            "target_c":self.target(),
            "cycles":self.cycles,
            "cycle":self.cycle,
            "stage":self.stage+1,
            "hold_remaining_s":self.hold_remaining,
            "latched":self.latched,
            "latch_rad":if self.latched{0.0}else{std::f64::consts::FRAC_PI_2},
            "lid_rad":if self.opened{self.config.opening_rad}else{0.0},
            "sample_visible":self.loaded,
            "running":self.active(),
            "aborted":matches!(self.phase,Phase::Cooling{aborted:true}|Phase::Complete{aborted:true}),
            "hot":self.hot(),
            "status":match self.phase{Phase::Idle=>"idle",Phase::Ramping=>"ramping",Phase::Holding=>"holding",Phase::Cooling{..}=>"cooling",Phase::Complete{aborted:true}=>"aborted",Phase::Complete{aborted:false}=>"complete"}
        })
    }
}
