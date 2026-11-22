use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    time::Duration,
};

use anyhow::{ensure, Result};
use osdl_core::EngineHandle;
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    net::TcpStream,
    task::JoinSet,
    time::{self, MissedTickBehavior},
};

use super::config::{worker_address, Id};

const PROBE_INTERVAL: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
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
    Tcp { address: SocketAddr },
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
        let Probe::Tcp { address } = config.probe;
        worker_address(address)?;
    }
    Ok(())
}

pub async fn start(configs: Vec<DeviceProbe>, engine: &EngineHandle) -> Result<JoinSet<()>> {
    let mut probes = JoinSet::new();
    for config in configs {
        engine
            .register_read_only_device(config.id.as_str(), &config.device_type, &config.description)
            .await
            .map_err(anyhow::Error::msg)?;
        probes.spawn(config.monitor(engine.clone()));
    }
    Ok(probes)
}

impl DeviceProbe {
    async fn monitor(self, engine: EngineHandle) {
        let mut interval = time::interval(PROBE_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut failed = false;
        loop {
            interval.tick().await;
            let result = time::timeout(PROBE_TIMEOUT, self.probe.sample())
                .await
                .map_err(std::io::Error::from)
                .and_then(|result| result);
            let properties = match result {
                Ok(properties) => Some(properties),
                Err(error) => {
                    if !failed {
                        log::warn!("device probe {} failed: {error}", self.id);
                    }
                    None
                }
            };
            failed = properties.is_none();
            engine
                .observe_read_only_device(self.id.as_str(), properties)
                .await;
        }
    }
}

impl Probe {
    async fn sample(&self) -> std::io::Result<Properties> {
        let Self::Tcp { address } = self;
        TcpStream::connect(address).await?;
        Ok(HashMap::from([("tcp_reachable".into(), Value::Bool(true))]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osdl_core::{protocol::DeviceCommand, OsdlEngine};
    use tokio::{io::AsyncReadExt, net::TcpListener};

    #[test]
    fn configured_probes_cannot_launch_programs() {
        let config = r#"
id: reader
device_type: example.instrument
description: Read-only instrument
probe:
  type: program
  executable: /usr/bin/true
"#;
        assert!(serde_yaml::from_str::<DeviceProbe>(config).is_err());
    }

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
        let update = |properties| engine.observe_read_only_device(id, properties);
        engine
            .register_read_only_device(id, "example.reader", "")
            .await
            .unwrap();
        assert!(!engine.get_device(id).await.unwrap().online);
        update(Some(HashMap::from([("ready".into(), Value::Bool(true))]))).await;
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
        update(None).await;
        let device = engine.get_device(id).await.unwrap();
        assert!(!device.online);
        assert!(device.properties.is_empty());
    }
}
