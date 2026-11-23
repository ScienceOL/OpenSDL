//! Full offline conformance validation for one portable instrument package.
//!
//! The pipeline has three explicit stages:
//! 1. `opensdl-assets` validates the manifest, portable file tree, GLB container,
//!    geometry references, and OpenSDL presentation declarations.
//! 2. `simulation_asset::prepare_validated` performs the same runtime admission
//!    used by managed worlds, including controller construction and initial
//!    GLB/runtime cross-checks.
//! 3. This module executes every declared action through the built-in dynamic
//!    conformance program, advances the deterministic controller clock, and
//!    validates every resulting observation.
//!
//! A successful report describes the built-in **kinematic** contract only. It
//! is not evidence of USD parity, multi-device scene behavior, or conformance
//! of a separately selected physics engine such as Isaac Sim or Genesis.

mod conformance;

use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Context, Result};
use opensdl_assets::{model::ModelDefinition, PackageDescriptor, VirtualAssetManifest};
use osdl_core::{
    config::SimulationDeviceConfig,
    simulation::{InstrumentState, SampleInventory},
};
use serde::Serialize;

use self::conformance::Step;

/// Machine-readable result returned by `lab validate --json`.
#[derive(Debug, Serialize)]
pub struct AssetValidationReport {
    /// Stable instrument package name from `asset.yaml`.
    pub name: String,
    /// Semantic version from `asset.yaml`.
    pub version: String,
    /// Selected runtime engine, or `None` for a presentation-only package.
    pub runtime: Option<String>,
    /// Dynamic evidence is present for one model-backed built-in instrument.
    /// Primitive-only example worlds are admitted but have no instrument model
    /// conformance report.
    pub dynamic: Option<DynamicValidationReport>,
}

/// Evidence collected while executing one instrument's built-in conformance
/// program. Counts are deterministic for a given package and OpenSDL version.
#[derive(Debug, Serialize)]
pub struct DynamicValidationReport {
    /// Runtime engine selected by the portable manifest.
    pub engine: String,
    /// Validated simulation device identifier.
    pub device_id: String,
    /// Shared GLB/runtime model identity.
    pub model_id: String,
    /// Number of distinct declared actions successfully exercised.
    pub actions_executed: usize,
    /// Number of complete observations cross-checked against the GLB contract.
    pub observations_checked: usize,
    /// Deterministic controller time advanced; no wall-clock sleep is used.
    pub simulated_seconds: f64,
}

/// Validates and packages one private snapshot of a working asset directory.
///
/// Packaging rereads metadata and source files, so validating the caller's
/// mutable tree first would leave a check/use gap. This owning boundary copies
/// the tree once, validates that exact snapshot, and hands only the private
/// snapshot to the lower-level OCI encoder. No daemon-side path from a mutable
/// directory is exposed: `lab build` imports the resulting verified layout.
pub fn pack_directory(directory: &Path, output: &Path) -> Result<PackageDescriptor> {
    let staging = tempfile::tempdir().context("cannot stage an asset validation snapshot")?;
    let snapshot = staging.path().join("asset");
    copy_asset_tree(directory, &snapshot).context("cannot snapshot the asset tree")?;
    validate_directory(&snapshot).context("cannot package an asset that failed validation")?;
    opensdl_assets::pack_directory(&snapshot, output)
        .context("cannot encode the validated asset snapshot")
}

fn copy_asset_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("cannot inspect {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "asset snapshots do not follow symbolic links: {}",
            source.display()
        );
    }
    if metadata.is_dir() {
        std::fs::create_dir(destination)
            .with_context(|| format!("cannot create {}", destination.display()))?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_asset_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        std::fs::copy(source, destination)
            .with_context(|| format!("cannot copy {}", source.display()))?;
    } else {
        bail!(
            "asset tree contains a non-regular file: {}",
            source.display()
        );
    }
    Ok(())
}

/// Runs static admission and dynamic behavior validation for a working asset
/// directory. The path may be anywhere on disk; no repository layout is assumed.
pub fn validate_directory(directory: &Path) -> Result<AssetValidationReport> {
    let asset = VirtualAssetManifest::load(directory).context("asset declaration stage failed")?;
    let simulation = crate::simulation_asset::prepare_validated(&asset, None, None)
        .context("runtime admission stage failed")?;
    let Some(runtime) = &asset.manifest.spec.runtime else {
        return Ok(AssetValidationReport {
            name: asset.manifest.metadata.name,
            version: asset.manifest.metadata.version,
            runtime: None,
            dynamic: None,
        });
    };
    let has_instrument = simulation
        .devices
        .iter()
        .any(|device| device.model.is_some());
    if !has_instrument {
        return Ok(AssetValidationReport {
            name: asset.manifest.metadata.name.clone(),
            version: asset.manifest.metadata.version.clone(),
            runtime: Some(format!("{:?}", runtime.engine).to_lowercase()),
            dynamic: None,
        });
    }
    let [device]: [SimulationDeviceConfig; 1] =
        simulation.devices.clone().try_into().map_err(|_| {
            anyhow::anyhow!("a model-backed asset package must declare exactly one device")
        })?;
    let model = device
        .model
        .as_ref()
        .context("a model-backed runtime must declare an instrument model")?;
    let definition = opensdl_assets::model::read_preview_model(&asset.paths.preview_model)?
        .context("a runnable instrument requires GLB model metadata")?;
    let dynamic = validate_dynamic(
        &device,
        model,
        &simulation.samples,
        &definition,
        &runtime.engine,
    )
    .context("dynamic behavior stage failed")?;
    Ok(AssetValidationReport {
        name: asset.manifest.metadata.name.clone(),
        version: asset.manifest.metadata.version.clone(),
        runtime: Some(dynamic.engine.clone()),
        dynamic: Some(dynamic),
    })
}

fn validate_dynamic(
    device: &SimulationDeviceConfig,
    model: &osdl_core::simulation::InstrumentConfig,
    sample_configs: &[osdl_core::simulation::SimulationSampleConfig],
    definition: &ModelDefinition,
    engine: &opensdl_assets::RuntimeEngine,
) -> Result<DynamicValidationReport> {
    let mut samples = SampleInventory::new(sample_configs).map_err(anyhow::Error::msg)?;
    let mut state = InstrumentState::new(model);
    let mut revision = 0_u64;
    let mut time_s = 0.0_f64;
    let mut observations_checked = 0;

    let mut observe = |state: &InstrumentState,
                       samples: &SampleInventory,
                       revision: u64,
                       time_s: f64|
     -> Result<()> {
        let properties = state.properties(&device.id, "validation", samples, revision, time_s);
        crate::simulation_contract::validate_observation(device, definition, &properties)?;
        observations_checked += 1;
        Ok(())
    };
    observe(&state, &samples, revision, time_s).context("initial dynamic observation")?;

    // Unknown actions must be rejected by the controller before a valid
    // program starts. Revision and time stay unchanged on this rejected call.
    let before = state.properties(&device.id, "validation", &samples, revision, time_s);
    if state
        .apply(
            &device.id,
            "__validation_unknown__",
            &serde_json::json!({}),
            &mut samples,
        )
        .is_ok()
    {
        bail!("controller accepted an undeclared unknown action");
    }
    let after = state.properties(&device.id, "validation", &samples, revision, time_s);
    if serde_json::to_value(before)? != serde_json::to_value(after)? {
        bail!("unknown action mutated the controller observation");
    }
    observe(&state, &samples, revision, time_s)?;

    let declared: HashSet<&str> = device
        .actions
        .iter()
        .map(|action| action.name.as_str())
        .collect();
    let mut executed = HashSet::new();
    for (index, step) in conformance::program(device, sample_configs)
        .into_iter()
        .enumerate()
    {
        match step {
            Step::Act(action, params) => {
                if action != "reset" && !declared.contains(action) {
                    bail!("conformance step {index} invokes undeclared action '{action}'");
                }
                if let Some(schema) = device
                    .actions
                    .iter()
                    .find(|candidate| candidate.name == action)
                {
                    crate::simulation_contract::validate_action_params(schema, &params)
                        .with_context(|| {
                            format!(
                                "conformance step {index} action '{action}' violates its declared schema"
                            )
                        })?;
                }
                state
                    .apply(&device.id, action, &params, &mut samples)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "conformance step {index} action '{action}' params {params} rejected: {error}"
                        )
                    })?;
                revision += 1;
                executed.insert(action);
                observe(&state, &samples, revision, time_s)
                    .with_context(|| format!("after conformance step {index} action '{action}'"))?;
            }
            Step::Advance(seconds) => {
                if !seconds.is_finite() || seconds <= 0.0 {
                    bail!("conformance step {index} has invalid duration {seconds}");
                }
                time_s += seconds;
                if state.advance(seconds) {
                    revision += 1;
                }
                observe(&state, &samples, revision, time_s).with_context(|| {
                    format!("after conformance step {index} advance {seconds}s")
                })?;
            }
        }
    }
    let mut missing: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|action| !executed.contains(action))
        .collect();
    missing.sort_unstable();
    if !missing.is_empty() {
        bail!(
            "dynamic conformance does not exercise declared actions: {}",
            missing.join(", ")
        );
    }

    Ok(DynamicValidationReport {
        engine: format!("{engine:?}").to_lowercase(),
        device_id: device.id.clone(),
        model_id: model.model_id().to_owned(),
        actions_executed: executed.len(),
        observations_checked,
        simulated_seconds: time_s,
    })
}
