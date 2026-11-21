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
use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Deserialize)]
pub struct SceneDefinition {
    pub name: String,
    pub instruments: Vec<SceneInstrument>,
}

impl SceneDefinition {
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        Ok(serde_json::from_reader(fs::File::open(path)?)?)
    }
}

#[derive(Clone, Deserialize)]
pub struct SceneInstrument {
    pub id: String,
    pub asset: String,
    pub version: String,
    pub position: [f64; 3],
    /// Local XYZ Euler angles in radians, relative to the parent or world.
    pub rotation: [f64; 3],
    pub parent: Option<String>,
}

pub fn prepare(
    cache: &AssetCache,
    digest: &str,
    destination: &Path,
) -> anyhow::Result<SimulationConfig> {
    let lease = cache.lock()?;
    let (asset, package) = lease.materialize_package(digest, destination)?;
    let preview = &package
        .layers
        .iter()
        .find(|layer| layer.media_type == opensdl_assets::PREVIEW_MEDIA_TYPE)
        .context("missing preview layer")?
        .digest;
    drop(lease);
    let Some(runtime) = asset.manifest.spec.runtime.as_ref() else {
        let metadata = asset.manifest.metadata;
        return Ok(SimulationConfig {
            devices: vec![osdl_core::config::SimulationDeviceConfig {
                id: metadata.name,
                device_type: "simulation.static".into(),
                description: metadata.display_name,
                role: None,
                model: None,
                actions: Vec::new(),
                asset_ref: None,
                position: [0.; 3],
                properties: std::collections::HashMap::from([
                    ("model_manifest_digest".into(), serde_json::json!(digest)),
                    ("model_preview_digest".into(), serde_json::json!(preview)),
                ]),
            }],
            ..Default::default()
        });
    };
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
        device
            .properties
            .insert("model_preview_digest".into(), serde_json::json!(preview));
        device
            .properties
            .insert("model_manifest_digest".into(), serde_json::json!(digest));
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
        }
    }
    Ok(simulation)
}

/// Build one world from independently admitted Hub packages. Only declarative
/// kinematic configurations are combined; no source code or hardware is loaded.
pub fn compose(
    cache: &AssetCache,
    instruments: &[SceneInstrument],
    destination: &Path,
    world_id: &str,
) -> anyhow::Result<SimulationConfig> {
    compose_packages(
        cache,
        instruments,
        destination,
        world_id,
        &mut Default::default(),
    )
}

type Templates = std::collections::HashMap<String, SimulationConfig>;
pub type SceneTemplates = std::sync::Arc<std::sync::Mutex<Templates>>;
fn compose_packages(
    cache: &AssetCache,
    instruments: &[SceneInstrument],
    destination: &Path,
    world_id: &str,
    templates: &mut Templates,
) -> anyhow::Result<SimulationConfig> {
    templates.retain(|digest, _| {
        instruments
            .iter()
            .any(|i| i.asset.rsplit_once('@').is_some_and(|(_, d)| d == digest))
    });
    if instruments.is_empty() || instruments.len() > 64 {
        bail!("select between 1 and 64 simulation packages");
    }
    let mut world = SimulationConfig {
        world_id: world_id.to_owned(),
        devices: Vec::new(),
        samples: Vec::new(),
        ..Default::default()
    };
    for (index, placement) in instruments.iter().enumerate() {
        let reference = &placement.asset;
        let parsed = opensdl_assets::AssetReference::parse(reference)?;
        let digest = parsed
            .digest()
            .context("simulation requires digest-pinned packages")?;
        let template = match templates.entry(digest.to_owned()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(
                prepare(cache, digest, &destination.join(index.to_string()))
                    .with_context(|| format!("cannot prepare simulation package {reference}"))?,
            ),
        };
        let package = template.clone();
        if index == 0 {
            world.tick_hz = package.tick_hz;
            world.seed = package.seed;
        } else {
            world.tick_hz = world.tick_hz.max(package.tick_hz);
        }
        for sample in package.samples {
            if let Some(existing) = world.samples.iter().find(|item| item.id == sample.id) {
                if serde_json::to_value(existing)? != serde_json::to_value(&sample)? {
                    bail!(
                        "packages declare conflicting sample fixture '{}'",
                        sample.id
                    );
                }
            } else {
                world.samples.push(sample);
            }
        }
        let [mut device]: [_; 1] = package.devices.try_into().map_err(|_| {
            anyhow::anyhow!("scene instrument must declare one device: {reference}")
        })?;
        let (namespace, name) = parsed
            .repository()
            .rsplit_once('/')
            .context("instrument asset has no namespace")?;
        device.asset_ref = Some(osdl_core::config::SimulationAssetRef {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            version: Some(placement.version.clone()),
        });
        device.properties.insert(
            "model_registry".into(),
            serde_json::json!(parsed.registry()),
        );
        device.id.clone_from(&placement.id);
        device.position = placement.position;
        device
            .properties
            .insert("rotation".into(), serde_json::json!(placement.rotation));
        device.properties.insert(
            "parent".into(),
            serde_json::json!(placement
                .parent
                .as_deref()
                .map(|id| osdl_core::transport::simulation::device_id_for(world_id, id))),
        );
        world.devices.push(device);
    }
    world.validate().map_err(anyhow::Error::msg)?;
    Ok(world)
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

/// Apply a Runner-owned revision from this instance's private data directory.
pub async fn apply_revision(
    engine: &osdl_core::EngineHandle,
    revision: u64,
    templates: SceneTemplates,
) -> anyhow::Result<()> {
    let config = engine.config();
    let world = &config
        .simulation
        .as_ref()
        .context("simulation unavailable")?
        .world_id;
    let root = config
        .data_dir
        .as_ref()
        .context("scene directory unavailable")?;
    let scene = SceneDefinition::read(&root.join(format!("scene-{revision}.json")))?;
    let devices = engine.list_devices().await;
    let ids: std::collections::HashMap<_, _> = devices
        .iter()
        .map(|d| {
            (
                d.id.as_str(),
                d.properties
                    .get("model_manifest_digest")
                    .and_then(|v| v.as_str()),
            )
        })
        .collect();
    let topology_changed = ids.len() != scene.instruments.len()
        || scene.instruments.iter().any(|i| {
            ids.get(osdl_core::transport::simulation::device_id_for(world, &i.id).as_str())
                .copied()
                .flatten()
                != i.asset.rsplit_once('@').map(|(_, d)| d)
        });
    let poses = scene
        .instruments
        .iter()
        .map(|i| osdl_core::simulation::world::DevicePose {
            id: i.id.clone(),
            position: i.position,
            rotation: i.rotation,
            parent: i
                .parent
                .as_ref()
                .map(|p| osdl_core::transport::simulation::device_id_for(world, p)),
        })
        .collect::<Vec<_>>();
    let next = if topology_changed {
        let root = root.clone();
        let world = world.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                let directory = tempfile::tempdir_in(&root)?;
                let mut templates = templates
                    .lock()
                    .map_err(|_| anyhow::anyhow!("scene unavailable"))?;
                compose_packages(
                    &AssetCache::open(root.join("assets"))?,
                    &scene.instruments,
                    directory.path(),
                    &world,
                    &mut templates,
                )
            })
            .await??,
        )
    } else {
        None
    };
    engine
        .apply_simulation_scene(next.as_ref(), &poses)
        .await
        .map_err(anyhow::Error::msg)
}
