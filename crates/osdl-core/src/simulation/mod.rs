pub mod balance;
pub mod basic;
pub mod world;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstrumentConfig {
    AnalyticalBalance(balance::BalanceConfig),
}
impl InstrumentConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::AnalyticalBalance(c) => c.validate(),
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
