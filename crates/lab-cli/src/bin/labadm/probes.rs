use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use osdl_core::EngineHandle;
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::AsyncReadExt,
    net::TcpStream,
    process::Command,
    task::JoinSet,
    time::{self, MissedTickBehavior},
};

use crate::config::Id;

const PROBE_INTERVAL: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_OUTPUT: u64 = 8 * 1024;
type Properties = HashMap<String, Value>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProbe {
    id: Id,
    device_type: String,
    description: String,
    probe: Probe,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Probe {
    Tcp {
        address: SocketAddr,
    },
    Program {
        executable: PathBuf,
        #[serde(default)]
        args: Vec<String>,
    },
}

pub fn validate(configs: &[DeviceProbe]) -> Result<()> {
    let mut seen = HashSet::new();
    for config in configs {
        ensure!(
            seen.insert(config.id.as_str()),
            "duplicate device ID {}",
            config.id
        );
        ensure!(
            !config.device_type.trim().is_empty(),
            "device type is empty for {}",
            config.id
        );
        match &config.probe {
            Probe::Tcp { address } => {
                crate::config::worker_address(*address)?;
            }
            Probe::Program { executable, .. } => ensure!(
                executable.is_absolute(),
                "probe executable for {} must be an absolute path",
                config.id
            ),
        }
    }
    Ok(())
}

pub async fn start(configs: Vec<DeviceProbe>, engine: &EngineHandle) -> Result<JoinSet<()>> {
    let mut probes = JoinSet::new();
    for config in configs {
        config.publish(engine, None).await?;
        probes.spawn(config.monitor(engine.clone()));
    }
    Ok(probes)
}

impl DeviceProbe {
    async fn monitor(self, engine: EngineHandle) {
        let mut interval = time::interval(PROBE_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let properties = match time::timeout(PROBE_TIMEOUT, self.probe.sample()).await {
                Ok(Ok(properties)) => Some(properties),
                Ok(Err(error)) => {
                    log::debug!("device probe {} failed: {error:#}", self.id);
                    None
                }
                Err(_) => {
                    log::debug!("device probe {} timed out", self.id);
                    None
                }
            };
            if let Err(error) = self.publish(&engine, properties).await {
                log::error!(
                    "device probe {} cannot update inventory: {error:#}",
                    self.id
                );
                return;
            }
        }
    }

    async fn publish(&self, engine: &EngineHandle, properties: Option<Properties>) -> Result<()> {
        engine
            .record_read_only_device(
                self.id.as_str(),
                &self.device_type,
                &self.description,
                properties,
            )
            .await
            .map_err(anyhow::Error::msg)
    }
}

impl Probe {
    async fn sample(&self) -> Result<Properties> {
        match self {
            Self::Tcp { address } => {
                TcpStream::connect(address).await?;
                Ok(HashMap::from([("tcp_reachable".into(), Value::Bool(true))]))
            }
            Self::Program { executable, args } => {
                let mut child = Command::new(executable)
                    .args(args)
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .stdout(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .context("start device probe executable")?;
                let mut output = Vec::new();
                child
                    .stdout
                    .take()
                    .context("missing probe stdout")?
                    .take(MAX_PROBE_OUTPUT + 1)
                    .read_to_end(&mut output)
                    .await?;
                ensure!(
                    output.len() as u64 <= MAX_PROBE_OUTPUT,
                    "probe output exceeded 8 KiB"
                );
                let status = child.wait().await?;
                ensure!(status.success(), "probe exited with {status}");
                serde_json::from_slice(&output).context("probe did not return a JSON object")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osdl_core::{protocol::DeviceCommand, OsdlEngine};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn tcp_probe_reports_reachability_without_sending_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let probe = Probe::Tcp { address };
        let properties = probe.sample().await.unwrap();
        assert_eq!(properties["tcp_reachable"], true);
        let (mut stream, _) = listener.accept().await.unwrap();
        assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
        drop(listener);
        assert!(probe.sample().await.is_err());
    }

    #[tokio::test]
    async fn device_observations_update_inventory_and_cannot_dispatch() {
        let engine = OsdlEngine::new(Default::default(), vec![]).handle();
        let id = "configured-reader";
        let update =
            |properties| engine.record_read_only_device(id, "example.reader", "", properties);
        update(None).await.unwrap();
        assert!(!engine.get_device(id).await.unwrap().online);
        update(Some(HashMap::from([("ready".into(), Value::Bool(true))])))
            .await
            .unwrap();
        let device = engine.get_device(id).await.unwrap();
        assert!(device.online);
        assert!(device.actions.is_empty());
        assert_eq!(device.properties["ready"], true);
        let error = engine
            .send_command(DeviceCommand {
                command_id: "test".into(),
                device_id: id.into(),
                action: "anything".into(),
                params: Value::Null,
            })
            .await
            .unwrap_err();
        assert!(error.contains("read-only"));
        update(None).await.unwrap();
        let device = engine.get_device(id).await.unwrap();
        assert!(!device.online);
        assert!(device.properties.is_empty());
    }
}
