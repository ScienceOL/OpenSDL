use osdl_core::{
    config::SimulationConfig,
    simulation::world::WorldRuntime,
    transport::{simulation::BuiltinSimulationBackendFactory, TransportRx},
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

fn setup() -> (
    std::sync::Arc<WorldRuntime>,
    mpsc::UnboundedReceiver<TransportRx>,
) {
    let config: SimulationConfig = serde_yaml::from_str(include_str!(
        "../../../assets/instruments/analytical-balance/package/source/world.yaml"
    ))
    .unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    (
        WorldRuntime::new(&config, tx, &BuiltinSimulationBackendFactory).unwrap(),
        rx,
    )
}
fn values(snapshot: &Value) -> &Value {
    &snapshot["properties"]["model_state"]["values"]
}

#[tokio::test]
async fn weighing_is_mass_conserving_and_rejections_do_not_emit_or_mutate() {
    let (world, mut rx) = setup();
    let id = "balance-1";
    assert_eq!(
        world
            .apply(id, "load_sample", &json!({"sample_id":"vessel"}))
            .await
            .unwrap_err(),
        "OPEN_DOOR_FIRST"
    );
    assert!(rx.try_recv().is_err());
    world.apply(id, "power", &json!({})).await.unwrap();
    world.advance(1.2).await;
    let snapshot = world.apply(id, "tare", &json!({})).await.unwrap();
    assert_eq!(values(&snapshot)["mass_g"], 0.0);
    assert_eq!(values(&snapshot)["stable"], true);
    world
        .apply(id, "set_door", &json!({"door":"left","opening_m":0.12}))
        .await
        .unwrap();
    let snapshot = world
        .apply(id, "load_sample", &json!({"sample_id":"vessel"}))
        .await
        .unwrap();
    assert_eq!(values(&snapshot)["mass_g"], 18.4);
    assert_eq!(values(&snapshot)["vessel_visible"], true);
    assert_eq!(
        world.apply(id, "tare", &json!({})).await.unwrap_err(),
        "WAIT_FOR_STABLE_LOAD"
    );
    assert_eq!(
        world
            .apply(id, "load_sample", &json!({"sample_id":"vessel"}))
            .await
            .unwrap_err(),
        "SAMPLE_ALREADY_PLACED"
    );
    assert_eq!(
        world
            .apply(id, "load_sample", &json!({"sample_id":"overload"}))
            .await
            .unwrap_err(),
        "PAN_OCCUPIED"
    );
    world
        .apply(id, "set_door", &json!({"door":"left","opening_m":0}))
        .await
        .unwrap();
    world.advance(1.1).await;
    assert!(world.apply(id, "tare", &json!({})).await.is_err());
    world.advance(0.1).await;
    let snapshot = world.apply(id, "tare", &json!({})).await.unwrap();
    assert_eq!(values(&snapshot)["mass_g"], 0.0);
    world
        .apply(id, "set_door", &json!({"door":"right","opening_m":0.12}))
        .await
        .unwrap();
    let snapshot = world
        .apply(id, "load_sample", &json!({"sample_id":"mass_5g"}))
        .await
        .unwrap();
    assert!((values(&snapshot)["mass_g"].as_f64().unwrap() - 5.0).abs() < 1e-9);
    world
        .apply(id, "unload_sample", &json!({"sample_id":"vessel"}))
        .await
        .unwrap();
    world
        .apply(id, "unload_sample", &json!({"sample_id":"mass_5g"}))
        .await
        .unwrap();
    let snapshot = world
        .apply(id, "load_sample", &json!({"sample_id":"overload"}))
        .await
        .unwrap();
    assert_eq!(values(&snapshot)["status"], "overload");
    assert!(values(&snapshot)["mass_g"].is_null());
    world.advance(10.0).await;
    assert!(world.apply(id, "tare", &json!({})).await.is_err());
    let snapshot = world.apply(id, "reset", &json!({})).await.unwrap();
    assert_eq!(values(&snapshot)["powered"], false);
    assert_eq!(values(&snapshot)["vessel_visible"], false);
    assert_eq!(values(&snapshot)["overload_visible"], false);
    while rx.try_recv().is_ok() {}
    for params in [
        json!({"door":"left","opening_m":0.121}),
        json!({"door":"front","opening_m":0}),
        json!({"door":"left","opening_m":0,"extra":true}),
    ] {
        assert!(world.apply(id, "set_door", &params).await.is_err());
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn samples_cannot_be_owned_by_two_devices_and_reset_releases_ownership() {
    let mut config: SimulationConfig = serde_yaml::from_str(include_str!(
        "../../../assets/instruments/analytical-balance/package/source/world.yaml"
    ))
    .unwrap();
    let mut second = config.devices[0].clone();
    second.id = "balance-2".into();
    config.devices.push(second);
    let (tx, _rx) = mpsc::unbounded_channel();
    let world = WorldRuntime::new(&config, tx, &BuiltinSimulationBackendFactory).unwrap();
    for id in ["balance-1", "balance-2"] {
        world
            .apply(id, "set_door", &json!({"door":"top","opening_m":0.12}))
            .await
            .unwrap();
    }
    world
        .apply("balance-1", "load_sample", &json!({"sample_id":"mass_20g"}))
        .await
        .unwrap();
    assert_eq!(
        world
            .apply("balance-2", "load_sample", &json!({"sample_id":"mass_20g"}))
            .await
            .unwrap_err(),
        "SAMPLE_ALREADY_PLACED"
    );
    world.apply("balance-1", "reset", &json!({})).await.unwrap();
    world
        .apply("balance-2", "load_sample", &json!({"sample_id":"mass_20g"}))
        .await
        .unwrap();
}

#[tokio::test]
async fn telemetry_cost_is_independent_of_prior_actions() {
    let (world, mut rx) = setup();
    for count in [100, 10_000] {
        let start = std::time::Instant::now();
        let mut bytes = 0;
        for i in 0..count {
            world
                .apply(
                    "balance-1",
                    "set_door",
                    &json!({"door":"left","opening_m":if i%2==0 {0.12} else {0.0}}),
                )
                .await
                .unwrap();
            bytes += rx.try_recv().unwrap().data.len();
        }
        eprintln!(
            "AX240 actions={count} mean_us={:.2} mean_bytes={:.0}",
            start.elapsed().as_secs_f64() * 1e6 / count as f64,
            bytes as f64 / count as f64
        );
        assert!(bytes / count < 1400);
        assert!(rx.try_recv().is_err());
    }
}
