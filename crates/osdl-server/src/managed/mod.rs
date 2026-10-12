//! Local lifecycle for digest-pinned, simulation-only instances.
mod control;
mod store;
pub mod worker;

use anyhow::{bail, Context};
use opensdl_assets::{AssetCache, RuntimeEngine};
use osdl_core::config::SimulationConfig;
use osdl_proto::v1 as pb;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use store::{idle, owned_directory, validate_name, Store};
pub use store::{InstanceInfo, InstanceState};

#[derive(Clone)]
pub struct ManagedHost {
    store: Store,
}
impl ManagedHost {
    pub fn open(cache: AssetCache) -> anyhow::Result<Self> {
        Ok(Self {
            store: Store::new(cache)?,
        })
    }
    pub fn create(&self, selector: &str, name: &str) -> anyhow::Result<InstanceInfo> {
        validate_name(name)?;
        let _command = self.store.command_lock(name)?;
        let lease = self.store.cache.lock()?;
        let digest = self.store.cache.resolve(selector)?;
        let package = self.store.cache.package(&digest)?;
        let runtime = package.asset.spec.runtime.as_ref().context(
            "asset has no executable runtime declaration; use a runnable kinematic package",
        )?;
        if runtime.engine != RuntimeEngine::Kinematic {
            bail!("runtime is unavailable");
        }
        let directory = self.store.directory(name);
        if directory.exists() {
            bail!("instance directory already exists; inspect/remove the retained instance first");
        }
        self.store.reserve(name, &digest)?;
        let prepared = (|| {
            owned_directory(&directory)?;
            let asset = lease.materialize(&digest, &directory.join("package"))?;
            let config_path = asset
                .paths
                .runtime_config
                .context("runtime config is missing")?;
            if fs::metadata(&config_path)?.len() > 1024 * 1024 {
                bail!("runtime config exceeds 1 MiB");
            }
            let simulation: SimulationConfig =
                serde_yaml::from_str(&fs::read_to_string(config_path)?)?;
            simulation.validate().map_err(anyhow::Error::msg)?;
            if simulation.engine != "kinematic" {
                bail!("runtime config requires unsupported engine");
            }
            self.store
                .prepared(name, &serde_yaml::to_string(&simulation)?)?;
            Ok(())
        })();
        if let Err(error) = prepared {
            self.store.settle(
                name,
                0,
                "failed",
                Some("invalid runtime preparation; remove the retained instance before retrying"),
            )?;
            return Err(error);
        }
        Ok(self.store.read(name)?.info)
    }
    pub async fn start(&self, name: &str, executable: &Path) -> anyhow::Result<InstanceInfo> {
        let _command = self.store.command_lock(name)?;
        let record = self.store.read(name)?;
        let process = self.store.process_lock(name)?;
        if !idle(&process)? {
            bail!("instance '{name}' already has a live worker");
        }
        if record.config.is_empty() {
            bail!("instance has no prepared runtime configuration; remove and recreate it");
        }
        let generation = self
            .store
            .generation(name, &uuid::Uuid::new_v4().simple().to_string())?;
        let directory = self.store.directory(name);
        store::regular_file(&directory.join("instance.log"))?;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("instance.log"))?;
        let mut command = Command::new(executable);
        command
            .args(["--cache-dir"])
            .arg(self.store.cache.root())
            .args([
                "instance-worker",
                "--name",
                name,
                "--generation",
                &generation.to_string(),
            ]);
        for variable in [
            "OSDL_ENDPOINT",
            "OSDL_INSTANCE",
            "OSDL_AUTH_TOKEN",
            "OSDL_CONFIG",
            "OSDL_SIMULATION",
            "LAB_REGISTRY_USERNAME",
            "LAB_REGISTRY_PASSWORD",
            "LAB_REGISTRY_TOKEN",
        ] {
            command.env_remove(variable);
        }
        command.env("RUST_LOG", "info");
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: a managed worker has no interactive console.
            command.creation_flags(0x08000000);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.store.settle(
                    name,
                    generation,
                    "failed",
                    Some("cannot spawn managed worker"),
                )?;
                return Err(error.into());
            }
        };
        let result = self.await_ready(name, generation, &mut child).await;
        if result.is_err() {
            let _ = child.kill();
            child.wait().context("reap failed managed startup")?;
            self.store.settle(
                name,
                generation,
                "failed",
                Some("startup failed; inspect retained instance logs"),
            )?;
        }
        result
    }

    async fn await_ready(
        &self,
        name: &str,
        generation: i64,
        child: &mut std::process::Child,
    ) -> anyhow::Result<InstanceInfo> {
        let directory = self.store.directory(name);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if child.try_wait()?.is_some() {
                bail!("worker exited during startup; inspect instance logs");
            }
            if let Some(identity) =
                crate::lockfile::find(&directory.join(format!("rpc-{generation}")), name)
            {
                if identity.pid == child.id() {
                    if let Some(addr) = identity.listen_addr {
                        let mut record = self.store.read(name)?;
                        record.info.pid = Some(child.id());
                        record.info.endpoint = Some(format!("http://{addr}"));
                        if let Ok(mut client) = control::connect(&record).await {
                            let status = client
                                .status(control::request(pb::StatusRequest {}, &record.token)?)
                                .await?
                                .into_inner();
                            if status.engine.is_some_and(|engine| engine.device_count > 0) {
                                self.store.running(
                                    name,
                                    generation,
                                    child.id(),
                                    record.info.endpoint.as_deref().unwrap(),
                                )?;
                                return Ok(self.store.read(name)?.info);
                            }
                        }
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("worker readiness timed out");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    pub fn inspect(&self, name: &str) -> anyhow::Result<InstanceInfo> {
        let _command = self.store.command_lock(name)?;
        let record = self.store.read(name)?;
        if matches!(
            record.info.state,
            InstanceState::Running | InstanceState::Starting | InstanceState::Stopping
        ) && idle(&self.store.process_lock(name)?)?
        {
            self.store.settle(
                name,
                record.info.generation,
                "failed",
                Some("worker is no longer running"),
            )?;
        }
        Ok(self.store.read(name)?.info)
    }
    pub fn list(&self, all: bool) -> anyhow::Result<Vec<InstanceInfo>> {
        let mut instances = Vec::new();
        for name in self.store.names()? {
            let info = self.inspect(&name)?;
            if all || matches!(info.state, InstanceState::Running) {
                instances.push(info);
            }
        }
        Ok(instances)
    }
    pub async fn stop(&self, name: &str) -> anyhow::Result<InstanceInfo> {
        let _command = self.store.command_lock(name)?;
        let record = self.store.read(name)?;
        let process = self.store.process_lock(name)?;
        if idle(&process)? {
            if matches!(
                record.info.state,
                InstanceState::Created | InstanceState::Exited | InstanceState::Failed
            ) {
                return Ok(record.info);
            }
            self.store.settle(
                name,
                record.info.generation,
                "failed",
                Some("worker stopped unexpectedly"),
            )?;
            return Ok(self.store.read(name)?.info);
        }
        let mut client = control::connect(&record)
            .await
            .context("cannot prove managed process identity; refusing shutdown")?;
        self.store.stopping(name, record.info.generation)?;
        client
            .shutdown(control::request(pb::ShutdownRequest {}, &record.token)?)
            .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while !idle(&process)? {
            if tokio::time::Instant::now() >= deadline {
                bail!("instance shutdown timed out; worker remains retained");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(self.store.read(name)?.info)
    }
    pub fn remove(&self, name: &str) -> anyhow::Result<()> {
        let _command = self.store.command_lock(name)?;
        let _lease = self.store.cache.lock()?;
        self.store.read(name)?;
        let process = self.store.process_lock(name)?;
        if !idle(&process)? {
            bail!("cannot remove a live instance; stop it first");
        }
        // Rename before unlink so the stored name never points into partially removed data.
        let directory = self.store.directory(name);
        let trash = self
            .store
            .cache
            .root()
            .join("instances")
            .join(format!(".removed-{}", uuid::Uuid::new_v4()));
        fs::rename(&directory, &trash)?;
        self.store.delete(name)?;
        drop(process);
        drop(_command);
        fs::remove_dir_all(&trash)?;
        Ok(())
    }
    pub fn logs(&self, name: &str, tail: usize) -> anyhow::Result<String> {
        if tail > 10000 {
            bail!("log tail must be at most 10000 lines");
        }
        let record = self.store.read(name)?;
        store::regular_file(&record.info.log_path)?;
        let file = match File::open(record.info.log_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(error) => return Err(error.into()),
        };
        let mut lines = std::collections::VecDeque::new();
        for line in BufReader::new(file).lines() {
            lines.push_back(line?);
            if lines.len() > tail {
                lines.pop_front();
            }
        }
        Ok(lines.into_iter().collect::<Vec<_>>().join("\n"))
    }
    pub async fn devices(&self, name: &str) -> anyhow::Result<serde_json::Value> {
        let record = self.store.read(name)?;
        let mut client = control::connect(&record).await?;
        let response = client
            .list_devices(control::request(
                pb::ListDevicesRequest {
                    adapter: None,
                    device_type: None,
                    role: None,
                },
                &record.token,
            )?)
            .await?
            .into_inner();
        Ok(serde_json::Value::Array(response.devices.into_iter().map(|device|serde_json::json!({"device_id":device.id,"device_type":device.device_type,"online":device.online,"properties":device.properties.as_ref().map(crate::convert::struct_to_json)})).collect()))
    }
    pub async fn send(
        &self,
        name: &str,
        device: &str,
        action: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        if !params.is_object() {
            bail!("action params must be a JSON object");
        }
        let record = self.store.read(name)?;
        let mut client = control::connect(&record).await?;
        let result = client
            .send_command(control::request(
                pb::SendCommandRequest {
                    command_id: String::new(),
                    device_id: device.to_owned(),
                    action: action.to_owned(),
                    params: Some(crate::convert::json_to_struct(&params)),
                },
                &record.token,
            )?)
            .await?
            .into_inner();
        Ok(
            serde_json::json!({"command_id":result.command_id,"device_id":result.device_id,"status":pb::command_result::Status::try_from(result.status)?.as_str_name(),"message":result.message}),
        )
    }
}
