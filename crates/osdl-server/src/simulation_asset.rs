//! One admission path for managed instances and supervised foreground runtimes.
use anyhow::{bail, Context};
use opensdl_assets::{
    model::{Control, ModelDefinition},
    AssetCache, RuntimeEngine,
};
use osdl_core::{
    config::SimulationConfig,
    simulation::{InstrumentConfig, InstrumentState, SampleInventory},
};
use std::{fs, path::Path};

pub fn prepare(
    cache: &AssetCache,
    digest: &str,
    destination: &Path,
) -> anyhow::Result<SimulationConfig> {
    let lease = cache.lock()?;
    let asset = lease.materialize(digest, destination)?;
    let runtime = asset
        .manifest
        .spec
        .runtime
        .as_ref()
        .context("asset has no executable runtime declaration")?;
    if runtime.engine != RuntimeEngine::Kinematic {
        bail!("runtime engine is unavailable");
    }
    let config = asset
        .paths
        .runtime_config
        .context("runtime config is missing")?;
    if fs::metadata(&config)?.len() > 1024 * 1024 {
        bail!("runtime config exceeds 1 MiB");
    }
    let mut simulation: SimulationConfig = serde_yaml::from_str(&fs::read_to_string(config)?)?;
    simulation.validate().map_err(anyhow::Error::msg)?;
    if simulation.engine != "kinematic" {
        bail!("runtime config requires unsupported engine");
    }
    let definition = opensdl_assets::model::read_preview_model(&asset.paths.preview_model)?;
    for device in &mut simulation.devices {
        if let Some(instrument) = &device.model {
            let definition = definition
                .as_ref()
                .context("runnable instrument requires model metadata")?;
            if definition.model_id != instrument.model_id() {
                bail!("runtime and preview model identities differ");
            }
            if let InstrumentConfig::AnalyticalBalance(balance) = instrument {
                validate_balance(definition, balance)?;
            }
            let samples = SampleInventory::new(&simulation.samples).map_err(anyhow::Error::msg)?;
            let properties = InstrumentState::new(instrument).properties(
                &device.id,
                "admission",
                &samples,
                0,
                0.0,
            );
            let values = &properties["model_state"]["values"];
            for joint in &definition.joints {
                let value = values[&joint.state]
                    .as_f64()
                    .context("joint observation is missing")?;
                if !matches!(joint.kind, opensdl_assets::model::JointKind::Continuous)
                    && !(joint.min..=joint.max).contains(&value)
                {
                    bail!("initial joint pose exceeds limits");
                }
                if let Some(state) = &joint.velocity_state {
                    if values[state].as_f64().is_none() {
                        bail!("joint velocity is missing");
                    }
                }
            }
            for binding in &definition.visibility {
                if !values[&binding.state].is_boolean() {
                    bail!("visibility observation is missing");
                }
            }
            for readout in &definition.readouts {
                if values
                    .get(&readout.state)
                    .is_none_or(|v| !v.is_null() && !v.is_number())
                {
                    bail!("readout observation is missing");
                }
            }

            for component in &definition.components {
                if let Some(scale) = &component.scale {
                    let value = values[&scale.state]
                        .as_f64()
                        .context("scale observation is missing")?;
                    if !(scale.min..=scale.max).contains(&value) {
                        bail!("initial scale exceeds limits");
                    }
                }
                let action = match &component.control {
                    Some(
                        Control::Command { action, .. }
                        | Control::Toggle { action, .. }
                        | Control::Dial { action, .. },
                    ) => action,
                    None => continue,
                };
                if !device
                    .actions
                    .iter()
                    .any(|declared| &declared.name == action)
                {
                    bail!("model component references an undeclared action");
                }
            }
            device
                .properties
                .insert("model_manifest_digest".into(), serde_json::json!(digest));
        }
    }
    Ok(simulation)
}
fn validate_balance(
    model: &ModelDefinition,
    balance: &osdl_core::simulation::balance::BalanceConfig,
) -> anyhow::Result<()> {
    if model.model_id != balance.model_id {
        bail!("runtime and preview model identities differ");
    }
    for door in ["left", "right", "top"] {
        let state = format!("door_{door}_m");
        let joint = model
            .joints
            .iter()
            .find(|j| j.state == state)
            .context("balance door binding is missing")?;
        if !matches!(joint.kind, opensdl_assets::model::JointKind::Prismatic)
            || joint.min != 0.0
            || joint.max != balance.door_travel_m
        {
            bail!("balance door limits differ from the model");
        }
    }
    let pan = model
        .placements
        .iter()
        .find(|p| p.id == "pan")
        .context("balance pan frame is missing")?;
    if pan.radius_m != balance.pan_radius_m {
        bail!("balance pan radius differs from the model");
    }
    Ok(())
}
