//! One admission path for managed instances and supervised foreground runtimes.
use anyhow::{bail, Context};
use opensdl_assets::{
    model::ModelDefinition, AssetCache, Capability, RuntimeEngine, ValidatedAsset,
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
    #[serde(default)]
    pub revision: u64,
    pub instruments: Vec<SceneInstrument>,
}

impl SceneDefinition {
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let scene: Self = serde_json::from_reader(fs::File::open(path)?)?;
        let parents: std::collections::HashMap<_, _> = scene
            .instruments
            .iter()
            .map(|instrument| (instrument.id.as_str(), instrument.parent.as_deref()))
            .collect();
        if parents.len() != scene.instruments.len() {
            bail!("scene instrument identities must be unique");
        }
        for &id in parents.keys() {
            let mut visited = std::collections::HashSet::new();
            let mut current = Some(id);
            while let Some(candidate) = current {
                if !visited.insert(candidate) {
                    bail!("cyclic scene parent for {id}");
                }
                current = *parents
                    .get(candidate)
                    .with_context(|| format!("unknown scene parent {candidate} for {id}"))?;
            }
        }
        Ok(scene)
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
    let preview = package
        .layers
        .iter()
        .find(|layer| layer.media_type == opensdl_assets::PREVIEW_MEDIA_TYPE)
        .context("missing preview layer")?
        .digest
        .clone();
    drop(lease);
    prepare_validated(&asset, Some(digest), Some(&preview))
}

/// Admits already validated portable paths without copying an OCI package.
///
/// Production preparation always supplies immutable content digests. The CLI
/// dynamic validator uses the same function with no digests because an
/// unpublished working directory has no content identity to project yet.
pub(crate) fn prepare_validated(
    asset: &ValidatedAsset,
    digest: Option<&str>,
    preview: Option<&str>,
) -> anyhow::Result<SimulationConfig> {
    let Some(runtime) = asset.manifest.spec.runtime.as_ref() else {
        let metadata = &asset.manifest.metadata;
        let mut properties = std::collections::HashMap::new();
        if let Some(digest) = digest {
            properties.insert("model_manifest_digest".into(), serde_json::json!(digest));
        }
        if let Some(preview) = preview {
            properties.insert("model_preview_digest".into(), serde_json::json!(preview));
        }
        return Ok(SimulationConfig {
            devices: vec![osdl_core::config::SimulationDeviceConfig {
                id: metadata.name.clone(),
                device_type: "simulation.static".into(),
                description: metadata.display_name.clone(),
                role: None,
                model: None,
                actions: Vec::new(),
                asset_ref: None,
                position: [0.; 3],
                properties,
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
        .as_ref()
        .context("runtime config is missing")?;
    if fs::metadata(config)?.len() > 1024 * 1024 {
        bail!("runtime config exceeds 1 MiB");
    }
    let mut simulation: SimulationConfig = serde_yaml::from_str(&fs::read_to_string(config)?)?;
    simulation.validate().map_err(anyhow::Error::msg)?;
    if simulation.engine != "kinematic" {
        bail!("runtime config requires unsupported engine");
    }
    let definition = opensdl_assets::model::read_preview_model(&asset.paths.preview_model)?;
    for device in &mut simulation.devices {
        if asset
            .manifest
            .spec
            .capabilities
            .contains(&Capability::OpensdlDevice)
            && !asset
                .manifest
                .spec
                .bindings
                .device_types
                .contains(&device.device_type)
        {
            bail!(
                "runtime device type '{}' is absent from spec.bindings.deviceTypes",
                device.device_type
            );
        }
        if let Some(preview) = preview {
            device
                .properties
                .insert("model_preview_digest".into(), serde_json::json!(preview));
        }
        if let Some(digest) = digest {
            device
                .properties
                .insert("model_manifest_digest".into(), serde_json::json!(digest));
        }
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
            crate::simulation_contract::validate_initial_contract(device, definition, &properties)?;
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

/// Admitted configuration for a staged revision. Composition never mutates the live world.
pub(crate) struct PreparedScene {
    pub(crate) ticket: uuid::Uuid,
    revision: u64,
    config: Option<SimulationConfig>,
    poses: Vec<osdl_core::simulation::world::DevicePose>,
}

impl PreparedScene {
    pub async fn apply(self, engine: &osdl_core::EngineHandle) -> Result<(), String> {
        engine
            .apply_simulation_scene(self.revision, self.config.as_ref(), &self.poses)
            .await
    }
}

pub(crate) async fn prepare_revision(
    engine: &osdl_core::EngineHandle,
    revision: u64,
    templates: SceneTemplates,
) -> anyhow::Result<PreparedScene> {
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
    if scene.revision != 0 && scene.revision != revision {
        bail!("scene revision does not match the requested revision");
    }
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
    Ok(PreparedScene {
        ticket: uuid::Uuid::new_v4(),
        revision,
        config: next,
        poses,
    })
}
