mod support;
use serde_json::{json, Value};
use support::instruments::{last, send, setup, v};

fn total_ml(state: &Value) -> f64 {
    ["source_ml", "receiver_ml", "line_ml", "removed_ml"]
        .iter()
        .map(|key| v(state)[key].as_f64().unwrap())
        .sum()
}

#[tokio::test]
async fn peristaltic_pump_accounts_for_prime_coast_and_liquid_removed_with_the_tubing() {
    let (world, mut rx, id) = setup("peristaltic-pump");
    send(&world, &id, "set_flow", json!({"flow_ml_min":56})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "load_tubing", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    assert_eq!(
        world.apply(&id, "start", &json!({})).await.unwrap_err(),
        "PRIME_FIRST"
    );
    let prime = send(&world, &id, "prime", json!({})).await;
    assert_eq!(v(&prime)["source_ml"], 93.0);
    assert_eq!(v(&prime)["line_ml"], 7.0);
    send(&world, &id, "start", json!({})).await;
    world.advance(4.0).await;
    let moving = last(&mut rx);
    assert!((v(&moving)["receiver_ml"].as_f64().unwrap() - 2.8 * 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(v(&moving)["rpm"], 20.0);
    assert!(
        (v(&moving)["roller_rad_s"].as_f64().unwrap() + 4.0 * 20.0 * std::f64::consts::TAU / 60.0)
            .abs()
            < 1e-9
    );
    assert!((total_ml(&moving) - 100.0).abs() < 1e-9);
    send(&world, &id, "stop", json!({})).await;
    assert_eq!(
        world.apply(&id, "lid", &json!({})).await.unwrap_err(),
        "MOTOR_MOVING"
    );
    world.advance(4.0).await;
    let stopped = last(&mut rx);
    assert!(
        v(&stopped)["receiver_ml"].as_f64().unwrap() > v(&moving)["receiver_ml"].as_f64().unwrap()
    );
    send(&world, &id, "lid", json!({})).await;
    let removed = send(&world, &id, "unload_tubing", json!({})).await;
    assert_eq!(v(&removed)["removed_ml"], 7.0);
    assert_eq!(v(&removed)["line_ml"], 0.0);
    assert!((total_ml(&removed) - 100.0).abs() < 1e-9);
}

#[tokio::test]
async fn capacity_boundary_stops_the_motor_at_the_same_time_in_large_and_small_steps() {
    let (coarse, mut crx, cid) = setup("peristaltic-pump");
    let (fine, mut frx, fid) = setup("peristaltic-pump");
    for (world, id) in [(&coarse, &cid), (&fine, &fid)] {
        for action in ["lid", "load_tubing", "lid", "prime"] {
            send(world, id, action, json!({})).await;
        }
        send(world, id, "set_flow", json!({"flow_ml_min":56})).await;
        send(world, id, "start", json!({})).await;
    }
    coarse.advance(1000.0).await;
    for _ in 0..4000 {
        fine.advance(0.25).await;
    }
    let a = last(&mut crx);
    let b = last(&mut frx);
    for state in [&a, &b] {
        assert_eq!(v(state)["source_ml"], 0.0);
        assert_eq!(v(state)["receiver_ml"], 93.0);
        assert_eq!(v(state)["rpm"], 0.0);
        assert_eq!(v(state)["status"], "empty");
        assert!((total_ml(state) - 100.0).abs() < 1e-8);
    }
    for key in ["rotor_rad", "roller_rad"] {
        assert!(
            (v(&a)[key].as_f64().unwrap() - v(&b)[key].as_f64().unwrap()).abs() < 1e-8,
            "{key}"
        );
    }
}

#[tokio::test]
async fn ultrasonic_cycle_requires_water_and_closes_on_its_timer_without_resetting_heat() {
    let (world, mut rx, id) = setup("ultrasonic-cleaner");
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "set_fill", json!({"fraction":0.25})).await;
    send(&world, &id, "load_basket", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    assert_eq!(
        world.apply(&id, "start", &json!({})).await.unwrap_err(),
        "FILL_TANK_FIRST"
    );
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_basket", json!({})).await;
    send(&world, &id, "set_fill", json!({"fraction":0.5})).await;
    send(&world, &id, "load_basket", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "set_temperature", json!({"temperature_c":60})).await;
    send(&world, &id, "set_duration", json!({"seconds":90})).await;
    send(&world, &id, "start", json!({})).await;
    assert_eq!(
        world.apply(&id, "lid", &json!({})).await.unwrap_err(),
        "STOP_FIRST"
    );
    world.advance(30.0).await;
    let running = last(&mut rx);
    assert_eq!(v(&running)["remaining_s"], 60.0);
    assert!(v(&running)["temperature_c"].as_f64().unwrap() > 22.0);
    world.advance(90.0).await;
    let complete = last(&mut rx);
    assert_eq!(v(&complete)["running"], false);
    assert_eq!(v(&complete)["remaining_s"], 0.0);
    assert!(v(&complete)["temperature_c"].as_f64().unwrap() > 22.0);
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_basket", json!({})).await;
    let drained = send(&world, &id, "drain", json!({})).await;
    assert_eq!(v(&drained)["fill_fraction"], 0.0);
    let filled = send(&world, &id, "set_fill", json!({"fraction":0.5})).await;
    assert!(
        v(&filled)["temperature_c"].as_f64().unwrap()
            < v(&drained)["temperature_c"].as_f64().unwrap()
    );
}
