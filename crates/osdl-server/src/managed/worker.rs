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
    let lock = store.process_lock(&name)?;
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
    let result = crate::serve(
        handle.clone(),
        crate::ServeConfig {
            instance: record.info.name.clone(),
            listen: crate::ListenConfig {
                socket_path: None,
                tcp_addr: Some("127.0.0.1:0".parse()?),
            },
            lock_dir,
            auth_token: Some(record.token.clone()),
        },
    )
    .await
    .context("managed gRPC server");
    handle.request_stop();
    task.await?;
    result
}
