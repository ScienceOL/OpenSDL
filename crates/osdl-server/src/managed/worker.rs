use super::store::Store;
use anyhow::{bail, Context};
use opensdl_assets::AssetCache;
use osdl_core::{
    adapter::simulation::SimulationAdapter,
    config::{OsdlConfig, SimulationConfig},
    EventStore, OsdlEngine,
};

pub async fn run(root: std::path::PathBuf, name: String, generation: i64) -> anyhow::Result<()> {
    let store = Store::new(AssetCache::open(root)?)?;
    let lock = store.worker_lock(&name)?;
    lock.lock()?;
    let record = store.read(&name)?;
    if record.info.generation != generation
        || !matches!(record.info.state, super::InstanceState::Starting)
    {
        bail!("worker start generation no longer owns the instance");
    }
    let result = serve(&store, &record, generation).await;
    let error = result
        .as_ref()
        .err()
        .map(|_| "managed worker failed; inspect its retained log");
    store.settle(
        &name,
        generation,
        if error.is_some() { "failed" } else { "exited" },
        error,
    )?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn a_worker_publishes_authenticated_readiness_without_its_initiator() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(AssetCache::open(temp.path()).unwrap()).unwrap();
        store
            .reserve("independent", &format!("sha256:{}", "a".repeat(64)))
            .unwrap();
        store
            .prepared("independent", "engine: kinematic\n")
            .unwrap();
        let generation = store
            .generation("independent", "private-test-token")
            .unwrap();
        drop(store.process_lock("independent").unwrap());
        let task = tokio::spawn(run(
            temp.path().to_owned(),
            "independent".into(),
            generation,
        ));
        let record = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let record = store.read("independent").unwrap();
                if matches!(record.info.state, super::super::InstanceState::Running) {
                    break record;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut client = super::super::control::connect(&record).await.unwrap();
        client
            .shutdown(
                super::super::control::request(osdl_proto::v1::ShutdownRequest {}, &record.token)
                    .unwrap(),
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            store.read("independent").unwrap().info.state,
            super::super::InstanceState::Exited
        ));
    }
}
async fn serve(
    store: &Store,
    record: &super::store::Record,
    generation: i64,
) -> anyhow::Result<()> {
    let simulation: SimulationConfig = serde_yaml::from_str(&record.config)?;
    simulation.validate().map_err(anyhow::Error::msg)?;
    if simulation.engine != "kinematic" {
        bail!("unsupported managed simulation engine");
    }
    super::store::owned_directory(&record.info.data_dir)?;
    super::store::regular_file(&record.info.data_dir.join("events.sqlite"))?;
    let events = EventStore::open(record.info.data_dir.join("events.sqlite").as_path())
        .map_err(anyhow::Error::msg)?;
    let mut engine = OsdlEngine::new(
        OsdlConfig {
            simulation: Some(simulation),
            data_dir: Some(record.info.data_dir.clone()),
            ..Default::default()
        },
        vec![Box::new(SimulationAdapter::new())],
    )
    .with_store(events);
    let handle = engine.handle();
    let task = tokio::spawn(async move { engine.run().await });
    // The existing server owns listener allocation, authentication, event shutdown,
    // and discovery cleanup. Only this private generation directory is discoverable.
    let lock_dir = store
        .directory(&record.info.name)
        .join(format!("rpc-{generation}"));
    let serving = crate::serve(
        handle.clone(),
        crate::ServeConfig {
            instance: record.info.name.clone(),
            listen: crate::ListenConfig {
                socket_path: None,
                tcp_addr: Some("127.0.0.1:0".parse()?),
            },
            lock_dir: lock_dir.clone(),
            auth_token: Some(record.token.clone()),
        },
    );
    tokio::pin!(serving);
    // The worker publishes its own readiness, so a hard exit of the initiating
    // CLI cannot leave a live process without a manageable authenticated endpoint.
    let ready = async {
        loop {
            if !handle.list_devices().await.is_empty() {
                if let Some(identity) = crate::lockfile::find(&lock_dir, &record.info.name) {
                    if identity.pid == std::process::id() {
                        if let Some(address) = identity.listen_addr {
                            let mut candidate = store.read(&record.info.name)?;
                            candidate.info.pid = Some(identity.pid);
                            candidate.info.endpoint = Some(format!("http://{address}"));
                            if super::control::ready(&candidate).await.unwrap_or(false) {
                                store.running(
                                    &record.info.name,
                                    generation,
                                    identity.pid,
                                    &format!("http://{address}"),
                                )?;
                                return Ok::<_, anyhow::Error>(());
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    let result = tokio::select! {
        result = &mut serving => result.context("managed gRPC server"),
        ready = tokio::time::timeout(std::time::Duration::from_secs(10), ready) => {
            match ready {
                Ok(Ok(())) => serving.await.context("managed gRPC server"),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(error.into()),
            }
        }
    };
    handle.request_stop();
    task.await?;
    result
}
