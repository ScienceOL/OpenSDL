//! Shared startup and lifetime for standalone and clustered OpenSDL engines.

use anyhow::{anyhow, Context};
use clap::Args;
use osdl_core::adapter::onvif::OnvifAdapter;
use osdl_core::adapter::simulation::SimulationAdapter;
use osdl_core::adapter::unilabos::UniLabOsAdapter;
use osdl_core::config::{
    AdapterConfig, EspNowDongleConfig, MqttConfig, OsdlConfig, SimulationConfig,
};
use osdl_core::driver::registry::DriverRegistry;
use osdl_core::path_expand;
use osdl_core::{EmbeddedBroker, EventStore, MdnsAdvertiser, OsdlEngine};
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct EngineArgs {
    /// YAML configuration for the local engine.
    #[arg(long, env = "OSDL_CONFIG")]
    pub config: Option<PathBuf>,

    /// Override the state dir (where the SQLite db lives).
    #[arg(long, env = "OSDL_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Override the registry directory for the unilabos adapter. Default
    /// pulls from `OSDL_REGISTRY_PATH` then `registry/unilabos`.
    #[arg(long, env = "OSDL_REGISTRY_PATH")]
    pub registry: Option<PathBuf>,

    /// ESP-NOW dongle port (USB-CDC). Optional.
    #[arg(long, env = "OSDL_DONGLE_PORT")]
    pub dongle_port: Option<String>,

    /// ESP-NOW dongle baud rate (default 115200).
    #[arg(long, env = "OSDL_DONGLE_BAUD", default_value_t = 115200)]
    pub dongle_baud: u32,

    /// Start a deterministic local simulation world with virtual devices.
    /// The world uses the same Lab Action Model and gRPC surface as physical
    /// devices, so local development does not require hardware.
    #[arg(long, env = "OSDL_SIMULATION")]
    pub simulation: bool,
    /// Add a digest-pinned package to this world. Repeat for multiple instruments.
    #[arg(long, conflicts_with_all = ["config", "registry", "dongle_port"], requires = "simulation")]
    pub asset: Vec<String>,
}

impl EngineArgs {
    pub fn expand_paths(&mut self) {
        fn expand_pathbuf(p: &Path) -> PathBuf {
            PathBuf::from(path_expand::expand_vars(&p.to_string_lossy()))
        }
        if let Some(p) = self.config.as_deref() {
            self.config = Some(expand_pathbuf(p));
        }
        if let Some(p) = self.registry.as_deref() {
            self.registry = Some(expand_pathbuf(p));
        }
        if let Some(p) = self.data_dir.as_deref() {
            self.data_dir = Some(expand_pathbuf(p));
        }
        if let Some(s) = self.dongle_port.as_deref() {
            // Dongle port is a path on Unix (`/dev/cu.*`) and a name
            // (`COM3`) on Windows. Either way passing it through expand_vars
            // is harmless: the Windows form has no `$` / `~` to substitute.
            let expanded = path_expand::expand_vars(s);
            if expanded != s {
                self.dongle_port = Some(expanded);
            }
        }
    }
}

pub enum EngineMode {
    Standalone { advertise_mqtt: bool },
    WorkPlane { advertise_mqtt: bool },
}

impl EngineMode {
    fn broker_addr(&self, config: &MqttConfig) -> anyhow::Result<SocketAddr> {
        let ip = match self {
            Self::Standalone { .. } => Ipv4Addr::UNSPECIFIED,
            Self::WorkPlane { .. } => {
                let ip = config.host.parse::<Ipv4Addr>().context(
                    "work-plane MQTT requires an explicit instrument interface IP in mqtt.host",
                )?;
                anyhow::ensure!(
                    !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast(),
                    "work-plane MQTT requires a unicast instrument interface"
                );
                ip
            }
        };
        Ok((ip, config.port).into())
    }

    fn advertise_mqtt(&self) -> bool {
        match *self {
            Self::Standalone { advertise_mqtt } | Self::WorkPlane { advertise_mqtt } => {
                advertise_mqtt
            }
        }
    }
}

pub async fn run_engine<F, S>(
    args: EngineArgs,
    instance: &str,
    default_data_dir: PathBuf,
    mode: EngineMode,
    serve: S,
) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>>,
    S: FnOnce(osdl_core::EngineHandle) -> F,
{
    anyhow::ensure!(
        osdl_auth::valid_instance_name(instance),
        "invalid engine instance name"
    );
    if matches!(mode, EngineMode::WorkPlane { .. }) {
        anyhow::ensure!(
            args.config.is_some() || args.simulation,
            "work-plane nodes require --config or --simulation"
        );
    }
    let mut config = build_config(&args)?;
    let broker_addr = config
        .mqtt
        .as_ref()
        .map(|mqtt| mode.broker_addr(mqtt))
        .transpose()?;
    let data_dir = args.data_dir.as_ref().unwrap_or(&default_data_dir);
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("create data dir {}", data_dir.display()))?;
    let lock_path = data_dir.join(format!("{instance}.engine.lock"));
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.try_lock()
        .with_context(|| format!("lock engine {}", lock_path.display()))?;

    let _asset_directory;
    if !args.asset.is_empty() {
        use opensdl_assets::{AssetCache, AssetReference, RegistryClient, RegistryCredentials};
        let cache = AssetCache::open(data_dir.join("assets"))?;
        if args.asset.len() > 64 {
            anyhow::bail!("select at most 64 simulation packages");
        }
        let references = args
            .asset
            .iter()
            .map(|reference| {
                let parsed = AssetReference::parse(reference)?;
                parsed
                    .digest()
                    .context("--asset requires a sha256 digest-pinned reference")?;
                Ok(parsed)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut pulls = tokio::task::JoinSet::new();
        let mut requested = std::collections::HashSet::new();
        for reference in references {
            let digest = reference.digest().expect("validated pinned reference");
            // A verified immutable cache hit needs no registry round trip.
            if !requested.insert(digest.to_owned()) || cache.package(digest).is_ok() {
                continue;
            }
            let cache = cache.clone();
            pulls.spawn(async move {
                RegistryClient::for_reference(&reference)
                    .pull(&reference, &cache, &RegistryCredentials::Anonymous, false)
                    .await
            });
            if pulls.len() == 4 {
                pulls
                    .join_next()
                    .await
                    .context("asset download task missing")???;
            }
        }
        while let Some(result) = pulls.join_next().await {
            result??;
        }
        _asset_directory = tempfile::tempdir_in(data_dir)?;
        config.simulation = Some(osdl_server::simulation_asset::compose(
            &cache,
            &args.asset,
            _asset_directory.path(),
            instance,
        )?);
    }
    config.data_dir = Some(data_dir.clone());

    // Start broker + mDNS only when MQTT is enabled in the config.
    let _broker = broker_addr
        .map(|addr| EmbeddedBroker::bind(addr).map_err(|e| anyhow!("broker: {e}")))
        .transpose()?;
    let _mdns = broker_addr
        .filter(|_| mode.advertise_mqtt())
        .map(|addr| MdnsAdvertiser::start(addr).map_err(|e| anyhow!("mdns: {e}")))
        .transpose()?;

    if config.mqtt.is_some() {
        // Give the broker a moment to bind before the engine connects.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let db_path = data_dir.join(format!("{instance}.db"));
    let store = EventStore::open(&db_path)
        .map_err(|e| anyhow!("event store {}: {}", db_path.display(), e))?;

    let adapters: Vec<Box<dyn osdl_core::adapter::ProtocolAdapter>> = vec![
        Box::new(UniLabOsAdapter::new(DriverRegistry::with_builtins())),
        Box::new(OnvifAdapter::new()),
        Box::new(SimulationAdapter::new()),
    ];
    let mut engine = OsdlEngine::new(config, adapters).with_store(store);
    let handle = engine.handle();

    // Engine runs in a dedicated task. We stop it explicitly after the
    // gRPC server returns so the shutdown order is: client → server → engine.
    let mut engine_task = tokio::spawn(async move { engine.run().await });

    // A dead engine must never leave an apparently healthy control service.
    let server = serve(handle.clone());
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => {
            handle.request_stop();
            engine_task.await.context("engine task failed")?;
            result.context("serve")
        }
        result = &mut engine_task => {
            handle.request_stop();
            // Release listeners, event streams and the instance lockfile.
            server.await.context("serve shutdown")?;
            result.context("engine task failed")?;
            anyhow::bail!("engine stopped before the control service")
        }
    }
}

fn build_config(args: &EngineArgs) -> anyhow::Result<OsdlConfig> {
    // Recipe-relative path resolution requires a stable base. Pick the
    // YAML's parent dir when a config was loaded — that's the only
    // reference point that travels with the file across machines (CWD
    // changes between an interactive shell and a launchd-spawned `.app`,
    // but `dirname(config.yaml)` does not). Fall back to the current
    // working directory when no config was given.
    let (mut cfg, config_dir) = if let Some(path) = &args.config {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: OsdlConfig = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse config {}", path.display()))?;
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        (cfg, dir)
    } else {
        // No config: ship the physical default unless simulation was
        // explicitly requested. A virtual lab must not probe a host's
        // physical registry or broker just because no recipe was supplied.
        let dir = std::env::current_dir().context("read current dir")?;
        let cfg = OsdlConfig {
            mqtt: if args.simulation {
                None
            } else {
                Some(MqttConfig::default())
            },
            adapters: if args.simulation {
                Vec::new()
            } else {
                vec![AdapterConfig {
                    adapter_type: "unilabos".into(),
                    registry_path: None,
                }]
            },
            ..Default::default()
        };
        (cfg, dir)
    };

    // Expand `~` / `$VAR` / `${VAR}` in every path-shaped field, and
    // resolve relatives against `config_dir`. Done before the CLI override
    // pass so a user-provided override always wins, even if the YAML had
    // a lab-local path with env vars.
    normalize_config_paths(&mut cfg, &config_dir);

    // CLI/env-var overrides take precedence over YAML so the same recipe
    // config can be shared across machines that have different dongle
    // ports or registry paths.
    if let Some(reg) = &args.registry {
        let reg_str = reg.display().to_string();
        if cfg.adapters.is_empty() {
            cfg.adapters.push(AdapterConfig {
                adapter_type: "unilabos".into(),
                registry_path: Some(reg_str),
            });
        } else {
            for a in &mut cfg.adapters {
                a.registry_path = Some(reg_str.clone());
            }
        }
    } else if !args.simulation {
        // Final fallback for adapters with no registry configured — pick
        // the registry that ships next to the config file. With a config
        // file, that's `<config_dir>/registry/unilabos`; without one,
        // it's the historical CWD-relative `registry/unilabos`.
        let default_registry = config_dir.join("registry/unilabos");
        for a in &mut cfg.adapters {
            if a.registry_path.is_none() {
                a.registry_path = Some(default_registry.display().to_string());
            }
        }
    }

    if let Some(port) = &args.dongle_port {
        if !port.is_empty() {
            // Replace any YAML-configured dongles: `--dongle-port` is the
            // authoritative answer for "which serial device is plugged in
            // *right now*", and shipping multiple at once would just
            // confuse the engine.
            if args.simulation {
                log::warn!("--simulation ignores --dongle-port; no physical hardware is started");
            } else {
                cfg.espnow_dongles = vec![EspNowDongleConfig {
                    port: port.clone(),
                    baud_rate: args.dongle_baud,
                }];
            }
        }
    }

    if args.simulation {
        // Preserve a world declared by the recipe. The flag is an enablement
        // shortcut, not a request to discard the user's device definitions.
        if cfg.simulation.is_none() {
            cfg.simulation = Some(SimulationConfig::default());
        }
        // A simulation process must not open a serial port from a recipe
        // copied from a physical lab. Use an explicit simulation recipe for
        // mixed deployments instead of silently combining the two modes.
        if !cfg.espnow_dongles.is_empty() {
            log::warn!("--simulation ignores ESP-NOW dongles from the recipe");
            cfg.espnow_dongles.clear();
        }
        if !cfg.qx96m_instruments.is_empty() {
            log::warn!("--simulation ignores QX96M instruments from the recipe");
            cfg.qx96m_instruments.clear();
        }
        if !cfg.sc9300_instruments.is_empty() {
            log::warn!("--simulation ignores SC9300 workstations from the recipe");
            cfg.sc9300_instruments.clear();
        }
    }

    Ok(cfg)
}

/// Expand env vars and resolve relative paths in every path-shaped config
/// field. Run **after** the YAML is parsed and **before** any CLI override
/// so user-provided overrides remain authoritative.
///
/// The fields touched here are all the spots where the engine treats a
/// string as a filesystem path: adapter registry directories, ESP-NOW
/// dongle device files, and the optional mediamtx binary path. URLs
/// (RTSP / mediamtx endpoints, MQTT host) are deliberately left alone.
fn normalize_config_paths(cfg: &mut OsdlConfig, config_dir: &Path) {
    for a in &mut cfg.adapters {
        if let Some(p) = a.registry_path.as_deref() {
            a.registry_path = Some(path_expand::expand(p, config_dir).display().to_string());
        }
    }
    for d in &mut cfg.espnow_dongles {
        // Dongle port: usually `/dev/cu.usbmodem...` (absolute) or `COM3`
        // (Windows, no leading separator). `expand` happens to leave
        // both alone because absolute paths short-circuit and
        // `COM3` has no `$` / `~` to expand. The point of running it
        // through is to support `${LAB_DONGLE_PORT}` in shared recipes.
        let raw = d.port.clone();
        let expanded = path_expand::expand(&raw, config_dir);
        d.port = expanded.display().to_string();
    }
    for instrument in &mut cfg.qx96m_instruments {
        instrument.key_file =
            path_expand::expand(&instrument.key_file.to_string_lossy(), config_dir);
    }
    if let Some(bin) = cfg.media_gateway.binary.as_deref() {
        cfg.media_gateway.binary = Some(path_expand::expand(&bin.to_string_lossy(), config_dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(simulation: bool, config: Option<PathBuf>) -> EngineArgs {
        EngineArgs {
            config,
            data_dir: None,
            registry: None,
            dongle_port: None,
            dongle_baud: 115_200,
            simulation,
            asset: Vec::new(),
        }
    }

    #[test]
    fn worker_broker_requires_specific_interface() {
        let mut mqtt = MqttConfig::default();
        let mode = EngineMode::WorkPlane {
            advertise_mqtt: false,
        };
        assert!(mode.broker_addr(&mqtt).is_err());
        mqtt.host = "0.0.0.0".into();
        assert!(mode.broker_addr(&mqtt).is_err());
        mqtt.host = "192.168.50.4".into();
        assert_eq!(
            mode.broker_addr(&mqtt).unwrap(),
            "192.168.50.4:1883".parse().unwrap()
        );
    }

    #[test]
    fn simulation_without_recipe_has_no_physical_defaults() {
        let config = build_config(&args(true, None)).expect("simulation config");

        assert!(config.mqtt.is_none());
        assert!(config.adapters.is_empty());
        assert!(config.espnow_dongles.is_empty());
        assert_eq!(
            config
                .simulation
                .expect("default simulation world")
                .world_id,
            "lab-sim"
        );
    }

    #[test]
    fn qx96m_key_paths_are_recipe_relative_and_simulation_opens_no_instruments() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("devices.yaml");
        std::fs::write(
            &path,
            "qx96m_instruments:\n  - id: test\n    endpoint: 192.168.42.10:6666\n    key_file: private/qx96m.key\n",
        )
        .unwrap();

        let physical = build_config(&args(false, Some(path.clone()))).unwrap();
        assert_eq!(
            physical.qx96m_instruments[0].key_file,
            directory.path().join("private/qx96m.key")
        );
        let simulation = build_config(&args(true, Some(path))).unwrap();
        assert!(simulation.qx96m_instruments.is_empty());
    }

    #[test]
    fn simulation_flag_preserves_recipe_world() {
        let path =
            std::env::temp_dir().join(format!("opensdl-simulation-{}.yaml", std::process::id()));
        std::fs::write(&path, "mqtt: null\nsimulation:\n  world_id: custom-world\n")
            .expect("write simulation recipe");

        let config = build_config(&args(true, Some(path.clone()))).expect("recipe config");
        let _ = std::fs::remove_file(path);

        assert_eq!(
            config.simulation.expect("recipe simulation world").world_id,
            "custom-world"
        );
    }
}
