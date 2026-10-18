use osdl_core::{
    config::SimulationConfig,
    simulation::world::WorldRuntime,
    transport::{simulation::BuiltinSimulationBackendFactory, TransportRx},
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

pub fn setup(
    slug: &str,
) -> (
    Arc<WorldRuntime>,
    mpsc::UnboundedReceiver<TransportRx>,
    String,
) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/instruments")
        .join(slug)
        .join("package/source/world.yaml");
    let config: SimulationConfig =
        serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let id = config.devices[0].id.clone();
    let (tx, rx) = mpsc::unbounded_channel();
    (
        WorldRuntime::new(&config, tx, &BuiltinSimulationBackendFactory).unwrap(),
        rx,
        id,
    )
}
pub fn v(snapshot: &Value) -> &Value {
    &snapshot["properties"]["model_state"]["values"]
}
pub async fn send(w: &WorldRuntime, id: &str, a: &str, p: Value) -> Value {
    w.apply(id, a, &p).await.unwrap()
}
pub fn last(rx: &mut mpsc::UnboundedReceiver<TransportRx>) -> Value {
    let mut value = None;
    while let Ok(rx) = rx.try_recv() {
        value = Some(serde_json::from_slice(&rx.data).unwrap());
    }
    value.unwrap()
}
