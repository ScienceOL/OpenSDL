mod support;
use serde_json::{json, Value};
use support::instruments::{last, send, setup, v};
fn total(state: &Value) -> f64 {
    ["source_ml", "receiver_ml", "removed_ml"]
        .iter()
        .map(|k| v(state)[k].as_f64().unwrap())
        .sum()
}

#[tokio::test]
async fn filtration_requires_a_sealed_filter_and_accounts_for_every_liquid_transfer() {
    let (world, mut rx, id) = setup("vacuum-filtration");
    assert_eq!(
        world.apply(&id, "start", &json!({})).await.unwrap_err(),
        "SEAL_FIRST"
    );
    send(&world, &id, "assemble", json!({})).await;
    assert_eq!(
        world
            .apply(&id, "load_slurry", &json!({}))
            .await
            .unwrap_err(),
        "INSTALL_FILTER_FIRST"
    );
    for action in ["install_filter", "load_slurry", "seal", "start"] {
        send(&world, &id, action, json!({})).await;
    }
    world.advance(10.0).await;
    let flowing = last(&mut rx);
    assert!(v(&flowing)["source_ml"].as_f64().unwrap() < 100.0);
    assert!(v(&flowing)["receiver_ml"].as_f64().unwrap() > 0.0);
    assert!(v(&flowing)["pressure_kpa"].as_f64().unwrap() < 40.0);
    assert!((total(&flowing) - 100.0).abs() < 1e-9);
    assert_eq!(
        world.apply(&id, "seal", &json!({})).await.unwrap_err(),
        "VENT_FIRST"
    );
    let stopped = send(&world, &id, "stop", json!({})).await;
    assert_eq!(v(&stopped)["pressure_kpa"], v(&flowing)["pressure_kpa"]);
    world.advance(2.0).await;
    let residual = last(&mut rx);
    assert!(
        v(&residual)["receiver_ml"].as_f64().unwrap()
            > v(&flowing)["receiver_ml"].as_f64().unwrap()
    );
    send(&world, &id, "vent", json!({})).await;
    world.advance(5.0).await;
    last(&mut rx);
    send(&world, &id, "seal", json!({})).await;
    send(&world, &id, "clear_slurry", json!({})).await;
    send(&world, &id, "assemble", json!({})).await;
    let empty = send(&world, &id, "empty_receiver", json!({})).await;
    assert!((v(&empty)["removed_ml"].as_f64().unwrap() - 100.0).abs() < 1e-9);
    assert_eq!(v(&empty)["source_ml"], 0.0);
    assert_eq!(v(&empty)["receiver_ml"], 0.0);
}

#[tokio::test]
async fn filtration_finishes_at_identical_inventory_under_coarse_and_fine_steps() {
    let (coarse, mut crx, cid) = setup("vacuum-filtration");
    let (fine, mut frx, fid) = setup("vacuum-filtration");
    for (world, id) in [(&coarse, &cid), (&fine, &fid)] {
        for action in ["assemble", "install_filter", "load_slurry", "seal", "start"] {
            send(world, id, action, json!({})).await;
        }
    }
    coarse.advance(120.0).await;
    for _ in 0..1200 {
        fine.advance(0.1).await;
    }
    for state in [last(&mut crx), last(&mut frx)] {
        assert_eq!(v(&state)["source_ml"], 0.0);
        assert!((v(&state)["receiver_ml"].as_f64().unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(v(&state)["running"], false);
        assert!((total(&state) - v(&state)["introduced_ml"].as_f64().unwrap()).abs() < 1e-9);
    }
}

#[tokio::test]
async fn rotavap_preserves_heat_and_vacuum_and_never_invents_evaporation() {
    let (world, mut rx, id) = setup("rotary-evaporator");
    send(&world, &id, "set_speed", json!({"rpm":120})).await;
    send(&world, &id, "load_flask", json!({})).await;
    assert_eq!(
        world.apply(&id, "lift", &json!({})).await.unwrap_err(),
        "CLAMP_FLASK_FIRST"
    );
    send(&world, &id, "clamp", json!({})).await;
    assert_eq!(
        world.apply(&id, "start", &json!({})).await.unwrap_err(),
        "LOWER_FLASK_FIRST"
    );
    send(&world, &id, "lift", json!({})).await;
    send(&world, &id, "set_temperature", json!({"temperature_c":60})).await;
    for action in ["start", "heat", "start_vacuum"] {
        send(&world, &id, action, json!({})).await;
    }
    world.advance(30.0).await;
    let moving = last(&mut rx);
    assert_eq!(v(&moving)["rpm"], 120.0);
    assert!(v(&moving)["pressure_kpa"].as_f64().unwrap() < 15.0);
    assert!(
        v(&moving)["temperature_c"].as_f64().unwrap() > v(&moving)["sample_c"].as_f64().unwrap()
    );
    assert_eq!(v(&moving)["sample_ml"], 200.0);
    assert_eq!(v(&moving)["evaporation_available"], false);
    let stop = send(&world, &id, "stop", json!({})).await;
    assert_eq!(v(&stop)["temperature_c"], v(&moving)["temperature_c"]);
    assert_eq!(v(&stop)["pressure_kpa"], v(&moving)["pressure_kpa"]);
    assert_eq!(
        world.apply(&id, "lift", &json!({})).await.unwrap_err(),
        "MOTOR_MOVING"
    );
    world.advance(3.0).await;
    last(&mut rx);
    assert_eq!(
        world.apply(&id, "lift", &json!({})).await.unwrap_err(),
        "VENT_FIRST"
    );
    send(&world, &id, "vent", json!({})).await;
    world.advance(5.0).await;
    last(&mut rx);
    send(&world, &id, "lift", json!({})).await;
    send(&world, &id, "clamp", json!({})).await;
    let unloaded = send(&world, &id, "unload_flask", json!({})).await;
    assert_eq!(v(&unloaded)["flask_visible"], false);
    assert!(v(&unloaded)["sample_c"].is_null());
}
