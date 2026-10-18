pub mod balance;
pub mod basic;
pub mod centrifuge;
pub mod chamber;
pub mod conductivity;
pub mod cycler;
mod motor;
pub mod orbital;
pub mod overhead;
pub mod peristaltic;
pub mod ph;
pub mod pipette;
pub mod refrigerator;
pub mod roller;
pub mod stirrer;
pub mod syringe;
pub mod thermal;
pub mod ultrasonic;
pub mod uvvis;
pub mod vacuum;
pub mod vortex;
pub mod world;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstrumentConfig {
    AnalyticalBalance(balance::BalanceConfig),
    HeatedStirrer(stirrer::StirrerConfig),
    WaterBath(chamber::ChamberConfig),
    DryingOven(chamber::ChamberConfig),
    DryBlockHeater(chamber::ChamberConfig),
    LaboratoryRefrigerator(refrigerator::RefrigeratorConfig),
    Centrifuge(centrifuge::CentrifugeConfig),
    VortexMixer(vortex::VortexConfig),
    OrbitalShaker(orbital::OrbitalConfig),
    TubeRoller(roller::RollerConfig),
    OverheadStirrer(overhead::OverheadConfig),
    UltrasonicCleaner(ultrasonic::UltrasonicConfig),
    PeristalticPump(peristaltic::PeristalticConfig),
    ConductivityMeter(conductivity::ConductivityConfig),
    UvVisSpectrophotometer(uvvis::UvVisConfig),
    ThermalCycler(cycler::CyclerConfig),
    Pipette(pipette::PipetteConfig),
    SyringePump(syringe::SyringeConfig),
    VacuumPump(vacuum::VacuumConfig),
    #[serde(rename = "ph_meter")]
    PHMeter(ph::PHConfig),
}
impl InstrumentConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::AnalyticalBalance(c) => c.validate(),
            Self::HeatedStirrer(c) => c.validate(),
            Self::WaterBath(c) => {
                c.validate()?;
                if c.max_c > 95.0 {
                    return Err("water bath excludes boiling; max_c must be at most 95".into());
                }
                Ok(())
            }
            Self::DryingOven(c) => c.validate(),
            Self::DryBlockHeater(c) => c.validate(),
            Self::LaboratoryRefrigerator(c) => c.validate(),
            Self::Centrifuge(c) => c.validate(),
            Self::VortexMixer(c) => c.validate(),
            Self::OrbitalShaker(c) => c.validate(),
            Self::TubeRoller(c) => c.validate(),
            Self::OverheadStirrer(c) => c.validate(),
            Self::UltrasonicCleaner(c) => c.validate(),
            Self::PeristalticPump(c) => c.validate(),
            Self::ConductivityMeter(c) => c.validate(),
            Self::UvVisSpectrophotometer(c) => c.validate(),
            Self::ThermalCycler(c) => c.validate(),
            Self::Pipette(c) => c.validate(),
            Self::SyringePump(c) => c.validate(),
            Self::VacuumPump(c) => c.validate(),
            Self::PHMeter(c) => c.validate(),
        }
    }
    pub fn model_id(&self) -> &str {
        match self {
            Self::AnalyticalBalance(c) => &c.model_id,
            Self::HeatedStirrer(c) => &c.model_id,
            Self::WaterBath(c) => &c.model_id,
            Self::DryingOven(c) => &c.model_id,
            Self::DryBlockHeater(c) => &c.model_id,
            Self::LaboratoryRefrigerator(c) => &c.model_id,
            Self::Centrifuge(c) => &c.model_id,
            Self::VortexMixer(c) => &c.model_id,
            Self::OrbitalShaker(c) => &c.model_id,
            Self::TubeRoller(c) => &c.model_id,
            Self::OverheadStirrer(c) => &c.model_id,
            Self::UltrasonicCleaner(c) => &c.model_id,
            Self::PeristalticPump(c) => &c.model_id,
            Self::ConductivityMeter(c) => &c.model_id,
            Self::UvVisSpectrophotometer(c) => &c.model_id,
            Self::ThermalCycler(c) => &c.model_id,
            Self::Pipette(c) => &c.model_id,
            Self::SyringePump(c) => &c.model_id,
            Self::VacuumPump(c) => &c.model_id,
            Self::PHMeter(c) => &c.model_id,
        }
    }
}

/// Fixture footprint in the pan's horizontal frame, in metres.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationSampleConfig {
    pub id: String,
    pub mass_kg: f64,
    pub position_m: [f64; 2],
    pub radius_m: f64,
}
#[derive(Debug)]
struct Sample {
    config: SimulationSampleConfig,
    owner: Option<String>,
}
#[derive(Debug, Default)]
pub struct SampleInventory {
    values: HashMap<String, Sample>,
}
impl SampleInventory {
    pub fn new(samples: &[SimulationSampleConfig]) -> Result<Self, String> {
        let mut inventory = Self::default();
        for sample in samples {
            if sample.id.is_empty()
                || !sample.mass_kg.is_finite()
                || sample.mass_kg <= 0.0
                || sample.position_m.iter().any(|n| !n.is_finite())
                || !sample.radius_m.is_finite()
                || sample.radius_m <= 0.0
                || inventory.values.contains_key(&sample.id)
            {
                return Err("invalid or duplicate sample fixture".into());
            }
            inventory.values.insert(
                sample.id.clone(),
                Sample {
                    config: sample.clone(),
                    owner: None,
                },
            );
        }
        Ok(inventory)
    }
    pub fn claim(&mut self, id: &str, device: &str, pan_radius_m: f64) -> Result<f64, String> {
        let sample = self.values.get(id).ok_or("UNKNOWN_SAMPLE")?;
        if sample.owner.is_some() {
            return Err("SAMPLE_ALREADY_PLACED".into());
        }
        let c = &sample.config;
        if c.position_m[0].hypot(c.position_m[1]) + c.radius_m > pan_radius_m {
            return Err("SAMPLE_OUTSIDE_PAN".into());
        }
        if self.values.values().any(|other| {
            other.owner.as_deref() == Some(device)
                && (c.position_m[0] - other.config.position_m[0])
                    .hypot(c.position_m[1] - other.config.position_m[1])
                    < c.radius_m + other.config.radius_m
        }) {
            return Err("PAN_OCCUPIED".into());
        }
        let mass = c.mass_kg;
        self.values.get_mut(id).expect("validated sample").owner = Some(device.to_owned());
        Ok(mass)
    }
    pub fn release(&mut self, id: &str, device: &str) -> Result<f64, String> {
        let sample = self.values.get_mut(id).ok_or("UNKNOWN_SAMPLE")?;
        if sample.owner.as_deref() != Some(device) {
            return Err("SAMPLE_NOT_ON_THIS_PAN".into());
        }
        sample.owner = None;
        Ok(sample.config.mass_kg)
    }
    pub fn release_device(&mut self, device: &str) {
        for sample in self.values.values_mut() {
            if sample.owner.as_deref() == Some(device) {
                sample.owner = None;
            }
        }
    }
    pub fn visibility<'a>(&'a self, device: &'a str) -> impl Iterator<Item = (&'a str, bool)> + 'a {
        self.values
            .iter()
            .map(move |(id, sample)| (id.as_str(), sample.owner.as_deref() == Some(device)))
    }
}

/// Closed instrument state; transports never interpret model-specific fields.
#[derive(Debug, Clone)]
pub enum InstrumentState {
    Balance(balance::BalanceState),
    Stirrer(stirrer::StirrerState),
    Chamber(chamber::ChamberState),
    Centrifuge(centrifuge::CentrifugeState),
    Vortex(vortex::VortexState),
    Orbital(orbital::OrbitalState),
    Roller(roller::RollerState),
    Refrigerator(refrigerator::RefrigeratorState),
    Overhead(overhead::OverheadState),
    Ultrasonic(ultrasonic::UltrasonicState),
    Peristaltic(peristaltic::PeristalticState),
    Conductivity(conductivity::ConductivityState),
    UvVis(uvvis::UvVisState),
    Cycler(cycler::CyclerState),
    Pipette(pipette::PipetteState),
    Syringe(syringe::SyringeState),
    Vacuum(vacuum::VacuumState),
    PH(ph::PHState),
}
impl InstrumentState {
    pub fn new(config: &InstrumentConfig) -> Self {
        match config {
            InstrumentConfig::AnalyticalBalance(c) => {
                Self::Balance(balance::BalanceState::new(c.clone()))
            }
            InstrumentConfig::HeatedStirrer(c) => {
                Self::Stirrer(stirrer::StirrerState::new(c.clone()))
            }
            InstrumentConfig::WaterBath(c) => Self::Chamber(chamber::ChamberState::new(c.clone())),
            InstrumentConfig::DryingOven(c) => Self::Chamber(chamber::ChamberState::new(c.clone())),
            InstrumentConfig::DryBlockHeater(c) => {
                Self::Chamber(chamber::ChamberState::new(c.clone()))
            }
            InstrumentConfig::LaboratoryRefrigerator(c) => {
                Self::Refrigerator(refrigerator::RefrigeratorState::new(c.clone()))
            }
            InstrumentConfig::Centrifuge(c) => {
                Self::Centrifuge(centrifuge::CentrifugeState::new(c.clone()))
            }
            InstrumentConfig::VortexMixer(c) => Self::Vortex(vortex::VortexState::new(c.clone())),
            InstrumentConfig::OrbitalShaker(c) => {
                Self::Orbital(orbital::OrbitalState::new(c.clone()))
            }
            InstrumentConfig::TubeRoller(c) => Self::Roller(roller::RollerState::new(c.clone())),
            InstrumentConfig::OverheadStirrer(c) => {
                Self::Overhead(overhead::OverheadState::new(c.clone()))
            }
            InstrumentConfig::UltrasonicCleaner(c) => {
                Self::Ultrasonic(ultrasonic::UltrasonicState::new(c.clone()))
            }
            InstrumentConfig::PeristalticPump(c) => {
                Self::Peristaltic(peristaltic::PeristalticState::new(c.clone()))
            }
            InstrumentConfig::ConductivityMeter(c) => {
                Self::Conductivity(conductivity::ConductivityState::new(c.clone()))
            }
            InstrumentConfig::UvVisSpectrophotometer(c) => {
                Self::UvVis(uvvis::UvVisState::new(c.clone()))
            }
            InstrumentConfig::ThermalCycler(c) => Self::Cycler(cycler::CyclerState::new(c.clone())),
            InstrumentConfig::Pipette(c) => Self::Pipette(pipette::PipetteState::new(c.clone())),
            InstrumentConfig::SyringePump(c) => {
                Self::Syringe(syringe::SyringeState::new(c.clone()))
            }
            InstrumentConfig::VacuumPump(c) => Self::Vacuum(vacuum::VacuumState::new(c.clone())),
            InstrumentConfig::PHMeter(c) => Self::PH(ph::PHState::new(c.clone())),
        }
    }
    pub fn apply(
        &mut self,
        device: &str,
        action: &str,
        params: &serde_json::Value,
        samples: &mut SampleInventory,
    ) -> Result<(), String> {
        match self {
            Self::Balance(s) => s.apply(device, action, params, samples),
            Self::Stirrer(s) => s.apply(device, action, params, samples),
            Self::Chamber(s) => s.apply(device, action, params, samples),
            Self::Centrifuge(s) => s.apply(device, action, params, samples),
            Self::Vortex(s) => s.apply(device, action, params, samples),
            Self::Orbital(s) => s.apply(device, action, params, samples),
            Self::Roller(s) => s.apply(device, action, params, samples),
            Self::Refrigerator(s) => s.apply(device, action, params, samples),
            Self::Overhead(s) => s.apply(device, action, params, samples),
            Self::Ultrasonic(s) => s.apply(device, action, params, samples),
            Self::Peristaltic(s) => s.apply(device, action, params, samples),
            Self::Conductivity(s) => s.apply(device, action, params, samples),
            Self::UvVis(s) => s.apply(device, action, params, samples),
            Self::Cycler(s) => s.apply(device, action, params, samples),
            Self::Pipette(s) => s.apply(device, action, params, samples),
            Self::Syringe(s) => s.apply(device, action, params, samples),
            Self::Vacuum(s) => s.apply(device, action, params, samples),
            Self::PH(s) => s.apply(device, action, params, samples),
        }
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        match self {
            Self::Balance(s) => s.advance(dt),
            Self::Stirrer(s) => s.advance(dt),
            Self::Chamber(s) => s.advance(dt),
            Self::Centrifuge(s) => s.advance(dt),
            Self::Vortex(s) => s.advance(dt),
            Self::Orbital(s) => s.advance(dt),
            Self::Roller(s) => s.advance(dt),
            Self::Refrigerator(s) => s.advance(dt),
            Self::Overhead(s) => s.advance(dt),
            Self::Ultrasonic(s) => s.advance(dt),
            Self::Peristaltic(s) => s.advance(dt),
            Self::Conductivity(s) => s.advance(dt),
            Self::UvVis(s) => s.advance(dt),
            Self::Cycler(s) => s.advance(dt),
            Self::Pipette(s) => s.advance(dt),
            Self::Syringe(s) => s.advance(dt),
            Self::Vacuum(s) => s.advance(dt),
            Self::PH(s) => s.advance(dt),
        }
    }
    pub fn properties(
        &self,
        device: &str,
        run: &str,
        samples: &SampleInventory,
        revision: u64,
        time: f64,
    ) -> HashMap<String, serde_json::Value> {
        let (model_id, values) = match self {
            Self::Balance(s) => return s.properties(device, run, samples, revision, time),
            Self::Stirrer(s) => (&s.config.model_id, s.values()),
            Self::Chamber(s) => (&s.config.model_id, s.values()),
            Self::Centrifuge(s) => (&s.config.model_id, s.values()),
            Self::Vortex(s) => (&s.config.model_id, s.values()),
            Self::Orbital(s) => (&s.config.model_id, s.values()),
            Self::Roller(s) => (&s.config.model_id, s.values()),
            Self::Refrigerator(s) => (&s.config.model_id, s.values()),
            Self::Overhead(s) => (&s.config.model_id, s.values()),
            Self::Ultrasonic(s) => (&s.config.model_id, s.values()),
            Self::Peristaltic(s) => (&s.config.model_id, s.values()),
            Self::Conductivity(s) => (&s.config.model_id, s.values()),
            Self::UvVis(s) => (&s.config.model_id, s.values()),
            Self::Cycler(s) => (&s.config.model_id, s.values()),
            Self::Pipette(s) => (&s.config.model_id, s.values()),
            Self::Syringe(s) => (&s.config.model_id, s.values()),
            Self::Vacuum(s) => (&s.config.model_id, s.values()),
            Self::PH(s) => (&s.config.model_id, s.values()),
        };
        HashMap::from([(
            "model_state".into(),
            serde_json::json!({"schema_version":1,"model_id":model_id,
            "run_id":run,"revision":revision,"time_s":time,"values":values}),
        )])
    }
}
