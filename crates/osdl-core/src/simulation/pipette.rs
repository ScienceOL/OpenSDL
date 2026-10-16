//! Original single-channel air-displacement teaching pipette; volumes in microlitres.
use super::{thermal, SampleInventory};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipetteConfig {
    pub model_id: String,
    pub min_ul: u32,
    pub max_ul: u32,
    pub stroke_m: f64,
    pub reservoir_ul: u32,
    pub source_x_m: f64,
    pub receiver_x_m: f64,
    pub immersion_m: f64,
}
impl PipetteConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.min_ul == 0
            || self.max_ul == 0
            || self.reservoir_ul == 0
            || !self.stroke_m.is_finite()
            || self.stroke_m <= 0.0
            || !self.source_x_m.is_finite()
            || !(-1.0..0.0).contains(&self.source_x_m)
            || !self.receiver_x_m.is_finite()
            || !(0.0..1.0).contains(&self.receiver_x_m)
            || !self.immersion_m.is_finite()
            || !(0.0..0.02).contains(&self.immersion_m)
            || self.min_ul > self.max_ul
            || self.reservoir_ul < self.max_ul
        {
            return Err("invalid pipette parameters".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Position {
    Park,
    Source,
    Receiver,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    position: Position,
}
#[derive(Debug, Clone)]
pub struct PipetteState {
    pub config: PipetteConfig,
    setting: u32,
    tip: bool,
    contained: u32,
    source: u32,
    receiver: u32,
    position: Position,
    pressed: bool,
}
impl PipetteState {
    pub fn new(config: PipetteConfig) -> Self {
        Self {
            setting: config.max_ul,
            source: config.reservoir_ul,
            config,
            tip: false,
            contained: 0,
            receiver: 0,
            position: Position::Park,
            pressed: false,
        }
    }
    pub fn apply(
        &mut self,
        _device: &str,
        action: &str,
        params: &Value,
        _samples: &mut SampleInventory,
    ) -> Result<(), String> {
        match action {
            "move_to" => {
                let p: Placement =
                    serde_json::from_value(params.clone()).map_err(|_| "INVALID_PARAMS")?;
                self.position = p.position;
                if self.position == Position::Park {
                    self.pressed = false;
                }
            }
            "set_volume" => {
                let value = thermal::number(
                    params,
                    "volume_ul",
                    self.config.min_ul as f64,
                    self.config.max_ul as f64,
                )?;
                if self.contained > 0 {
                    return Err("DISPENSE_FIRST".into());
                }
                if value.fract() != 0.0 {
                    return Err("INVALID_PARAMS".into());
                }
                self.setting = value as u32;
            }
            "tip" => {
                thermal::empty(params)?;
                if self.contained > 0 {
                    return Err("DISPENSE_FIRST".into());
                }
                if self.position != Position::Park {
                    return Err("PARK_PIPETTE_FIRST".into());
                }
                self.tip = !self.tip;
            }
            "aspirate" | "dispense" | "stroke" => {
                thermal::empty(params)?;
                if !self.tip {
                    return Err("ATTACH_TIP_FIRST".into());
                }
                let aspirate = action == "aspirate" || (action == "stroke" && self.contained == 0);
                if aspirate {
                    if self.position != Position::Source {
                        return Err("IMMERSE_SOURCE_FIRST".into());
                    }
                    if self.contained > 0 {
                        return Err("DISPENSE_FIRST".into());
                    }
                    if self.source < self.setting {
                        return Err("INSUFFICIENT_VOLUME".into());
                    }
                    self.source -= self.setting;
                    self.contained = self.setting;
                    self.pressed = false;
                } else {
                    if self.contained == 0 {
                        return Err("ASPIRATE_FIRST".into());
                    }
                    if self.position != Position::Receiver {
                        return Err("POSITION_RECEIVER_FIRST".into());
                    }
                    self.receiver += self.contained;
                    self.contained = 0;
                    self.pressed = true;
                }
            }
            "reset" => {
                thermal::empty(params)?;
                *self = Self::new(self.config.clone());
            }
            _ => return Err("UNKNOWN_ACTION".into()),
        }
        Ok(())
    }
    pub fn advance(&mut self, _dt: f64) -> bool {
        false
    }
    pub fn values(&self) -> Value {
        json!({
            "pipette_position":match self.position {Position::Park=>"park",Position::Source=>"source",Position::Receiver=>"receiver"},
            "pipette_x_m":match self.position {Position::Park=>0.0,Position::Source=>self.config.source_x_m,Position::Receiver=>self.config.receiver_x_m},
            "pipette_vertical_m":if self.position==Position::Source {-self.config.immersion_m}else{0.0},
            "source_fraction":self.source as f64/self.config.reservoir_ul as f64,
            "receiver_fraction":self.receiver as f64/self.config.reservoir_ul as f64,
            "source_liquid_visible":self.source>0,
            "receiver_liquid_visible":self.receiver>0,
            "volume_ul":self.setting,
            "tip_ul":self.contained,
            "source_ul":self.source,
            "receiver_ul":self.receiver,
            "tip_visible":self.tip,
            "fill_fraction":self.contained as f64/self.config.max_ul as f64,
            "liquid_visible":self.contained>0,
            "plunger_m":if self.pressed {self.config.stroke_m*self.setting as f64/self.config.max_ul as f64}else{0.0},
            "volume_dial_rad":self.setting as f64/self.config.max_ul as f64*std::f64::consts::TAU*0.75,
            "status":if !self.tip {"no_tip"}else if self.contained>0 {"loaded"}else{"ready"}})
    }
}
