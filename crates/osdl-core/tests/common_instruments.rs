use osdl_core::{
    config::SimulationConfig,
    simulation::world::WorldRuntime,
    transport::{simulation::BuiltinSimulationBackendFactory, TransportRx},
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
fn setup(
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
fn v(snapshot: &Value) -> &Value {
    &snapshot["properties"]["model_state"]["values"]
}
async fn send(w: &WorldRuntime, id: &str, a: &str, p: Value) -> Value {
    w.apply(id, a, &p).await.unwrap()
}
fn last(rx: &mut mpsc::UnboundedReceiver<TransportRx>) -> Value {
    let mut value = None;
    while let Ok(rx) = rx.try_recv() {
        value = Some(serde_json::from_slice(&rx.data).unwrap());
    }
    value.unwrap()
}
#[tokio::test]
async fn centrifuge_rejects_imbalance_and_keeps_lid_locked_through_deceleration() {
    let (w, mut rx, id) = setup("centrifuge");
    send(&w, &id, "set_speed", json!({"rpm":2000})).await;
    send(&w, &id, "lid", json!({})).await;
    send(&w, &id, "load_tube", json!({"slot":0})).await;
    send(&w, &id, "lid", json!({})).await;
    assert_eq!(
        w.apply(&id, "start", &json!({})).await.unwrap_err(),
        "ROTOR_IMBALANCE"
    );
    send(&w, &id, "lid", json!({})).await;
    send(&w, &id, "load_tube", json!({"slot":2})).await;
    send(&w, &id, "lid", json!({})).await;
    send(&w, &id, "set_speed", json!({"rpm":2000})).await;
    send(&w, &id, "start", json!({})).await;
    w.advance(2.0).await;
    assert_eq!(v(&last(&mut rx))["rpm"], 1000.0);
    assert_eq!(
        w.apply(&id, "lid", &json!({})).await.unwrap_err(),
        "ROTOR_MOVING"
    );
    send(&w, &id, "stop", json!({})).await;
    w.advance(1.0).await;
    assert_eq!(
        w.apply(&id, "lid", &json!({})).await.unwrap_err(),
        "ROTOR_MOVING"
    );
    w.advance(1.0).await;
    let snapshot = send(&w, &id, "lid", json!({})).await;
    assert_eq!(v(&snapshot)["rpm"], 0.0);
    assert!(v(&snapshot)["lid_rad"].as_f64().unwrap() > 1.0);
}
#[tokio::test]
async fn pipette_and_syringe_conserve_volume_and_stop_at_capacity() {
    let (w, _rx, id) = setup("pipette");
    assert_eq!(
        w.apply(&id, "aspirate", &json!({})).await.unwrap_err(),
        "ATTACH_TIP_FIRST"
    );
    send(&w, &id, "tip", json!({})).await;
    send(&w, &id, "set_volume", json!({"volume_ul":100})).await;
    send(&w, &id, "move_to", json!({"position":"source"})).await;
    let a = send(&w, &id, "aspirate", json!({})).await;
    assert_eq!(v(&a)["tip_ul"], 100);
    assert_eq!(
        w.apply(&id, "tip", &json!({})).await.unwrap_err(),
        "DISPENSE_FIRST"
    );
    send(&w, &id, "move_to", json!({"position":"receiver"})).await;
    let b = send(&w, &id, "dispense", json!({})).await;
    assert_eq!(
        v(&b)["source_ul"].as_f64().unwrap()
            + v(&b)["tip_ul"].as_f64().unwrap()
            + v(&b)["receiver_ul"].as_f64().unwrap(),
        2000.0
    );
    let (w, mut rx, id) = setup("syringe-pump");
    send(&w, &id, "set_flow", json!({"flow_ml_min":10})).await;
    send(&w, &id, "start", json!({})).await;
    w.advance(30.0).await;
    let b = last(&mut rx);
    assert!((v(&b)["delivered_ml"].as_f64().unwrap() - 5.0).abs() < 1e-9);
    w.advance(100.0).await;
    let b = last(&mut rx);
    assert_eq!(v(&b)["remaining_ml"], 0.0);
    assert_eq!(v(&b)["running"], false);
    assert!(v(&b)["plunger_m"].as_f64().unwrap() <= 0.065);
    assert_eq!(
        v(&b)["source_ml"].as_f64().unwrap()
            + v(&b)["remaining_ml"].as_f64().unwrap()
            + v(&b)["delivered_ml"].as_f64().unwrap(),
        20.0
    );
}
#[tokio::test]
async fn bath_and_oven_have_independent_sample_temperature_and_cool_when_open() {
    for slug in ["water-bath", "drying-oven"] {
        let (w, mut rx, id) = setup(slug);
        send(&w, &id, "lid", json!({})).await;
        send(&w, &id, "load_sample", json!({})).await;
        assert_eq!(
            w.apply(&id, "start", &json!({})).await.unwrap_err(),
            "CLOSE_LID_FIRST"
        );
        send(&w, &id, "lid", json!({})).await;
        send(&w, &id, "set_temperature", json!({"temperature_c":60})).await;
        send(&w, &id, "start", json!({})).await;
        w.advance(30.0).await;
        let snapshot = last(&mut rx);
        let hot = v(&snapshot)["temperature_c"].as_f64().unwrap();
        let sample = v(&snapshot)["sample_c"].as_f64().unwrap();
        assert!(hot > sample && sample > 22.0);
        send(&w, &id, "lid", json!({})).await;
        w.advance(10.0).await;
        let snapshot = last(&mut rx);
        assert_eq!(v(&snapshot)["running"], false);
        assert!(v(&snapshot)["temperature_c"].as_f64().unwrap() < hot);
    }
}
#[tokio::test]
async fn vortex_pump_and_ph_follow_declared_time_evolution() {
    let (w, mut rx, id) = setup("vortex-mixer");
    send(&w, &id, "set_speed", json!({"rpm":1500})).await;
    assert_eq!(
        w.apply(&id, "start", &json!({})).await.unwrap_err(),
        "LOAD_SAMPLE_FIRST"
    );
    send(&w, &id, "load_tube", json!({})).await;
    send(&w, &id, "set_speed", json!({"rpm":1500})).await;
    send(&w, &id, "start", json!({})).await;
    w.advance(1.0).await;
    assert_eq!(v(&last(&mut rx))["rpm"], 800.0);
    send(&w, &id, "stop", json!({})).await;
    w.advance(1.0).await;
    assert_eq!(v(&last(&mut rx))["rpm"], 0.0);
    let (w, mut rx, id) = setup("vacuum-pump");
    send(&w, &id, "start", json!({})).await;
    w.advance(25.0).await;
    let b = last(&mut rx);
    let low = v(&b)["pressure_kpa"].as_f64().unwrap();
    assert!(low > 2.0 && low < 50.0);
    send(&w, &id, "vent", json!({})).await;
    assert_eq!(
        w.apply(&id, "start", &json!({})).await.unwrap_err(),
        "CLOSE_VENT_FIRST"
    );
    w.advance(2.0).await;
    assert!(v(&last(&mut rx))["pressure_kpa"].as_f64().unwrap() > low);
    let (w, mut rx, id) = setup("ph-meter");
    send(&w, &id, "power", json!({})).await;
    send(&w, &id, "set_buffer", json!({"ph":4})).await;
    send(&w, &id, "probe", json!({})).await;
    w.advance(2.0).await;
    let b = last(&mut rx);
    let ph = v(&b)["ph"].as_f64().unwrap();
    assert!((ph - (4.0 + 3.0 / std::f64::consts::E)).abs() < 1e-9);
    assert_eq!(
        w.apply(&id, "set_buffer", &json!({"ph":7}))
            .await
            .unwrap_err(),
        "RAISE_PROBE_FIRST"
    );
    send(&w, &id, "probe", json!({})).await;
    send(&w, &id, "set_buffer", json!({"ph":7})).await;
    send(&w, &id, "probe", json!({})).await;
    w.advance(20.0).await;
    let b = send(&w, &id, "calibrate", json!({})).await;
    assert_eq!(v(&b)["calibrated"], true);
}

#[tokio::test]
async fn current_command_latency_and_bytes_do_not_grow_with_history() {
    for history in [100usize, 10_000, 100_000] {
        let (w, mut rx, id) = setup("heated-stirrer");
        for i in 0..history {
            send(&w, &id, "set_speed", json!({"rpm":i%1501})).await;
            rx.try_recv().unwrap();
        }
        let mut times = Vec::with_capacity(1000);
        let mut bytes = 0;
        for i in 0..1000 {
            let start = std::time::Instant::now();
            send(&w, &id, "set_speed", json!({"rpm":i%1501})).await;
            times.push(start.elapsed().as_nanos());
            bytes += rx.try_recv().unwrap().data.len();
        }
        times.sort_unstable();
        eprintln!(
            "HS150 history={history} n=1000 p50_us={:.2} p95_us={:.2} mean_bytes={}",
            times[500] as f64 / 1000.0,
            times[950] as f64 / 1000.0,
            bytes / 1000
        );
        assert!(bytes / 1000 < 2048);
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commands_remain_scoped_and_telemetry_is_bounded() {
    let original: SimulationConfig = serde_yaml::from_str(include_str!(
        "../../../assets/instruments/heated-stirrer/package/source/world.yaml"
    ))
    .unwrap();
    for count in [1, 3, 12] {
        let mut config = original.clone();
        config.devices = (0..count)
            .map(|i| {
                let mut d = original.devices[0].clone();
                d.id = format!("stirrer-{i}");
                d
            })
            .collect();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let world = WorldRuntime::new(&config, tx, &BuiltinSimulationBackendFactory).unwrap();
        let drain = tokio::spawn(async move {
            let mut bytes = 0;
            for _ in 0..count * 1000 {
                bytes += rx.recv().await.unwrap().data.len();
            }
            bytes
        });
        let mut tasks = tokio::task::JoinSet::new();
        for device in &config.devices {
            let world = world.clone();
            let id = device.id.clone();
            tasks.spawn(async move {
                let mut samples = Vec::with_capacity(1000);
                for n in 0..1000 {
                    let start = std::time::Instant::now();
                    let snapshot = world
                        .apply(&id, "set_speed", &json!({"rpm":n}))
                        .await
                        .unwrap();
                    samples.push(start.elapsed().as_nanos());
                    assert_eq!(v(&snapshot)["target_rpm"], n as f64);
                }
                samples
            });
        }
        let mut times = Vec::new();
        while let Some(result) = tasks.join_next().await {
            times.extend(result.unwrap());
        }
        times.sort_unstable();
        let bytes = drain.await.unwrap() / (count * 1000);
        eprintln!(
            "HS150 concurrent={count} n={} p50_us={:.2} p95_us={:.2} mean_bytes={bytes}",
            times.len(),
            times[times.len() / 2] as f64 / 1000.0,
            times[times.len() * 95 / 100] as f64 / 1000.0
        );
        assert!(bytes < 2048);
    }
}

#[tokio::test]
async fn active_thermal_motor_tick_cost() {
    let (world, mut rx, id) = setup("heated-stirrer");
    for action in ["load_vessel", "power", "probe"] {
        send(&world, &id, action, json!({})).await;
    }
    send(&world, &id, "set_temperature", json!({"temperature_c":80})).await;
    send(&world, &id, "set_speed", json!({"rpm":900})).await;
    for action in ["heat", "stir"] {
        send(&world, &id, action, json!({})).await;
    }
    while rx.try_recv().is_ok() {}
    let mut times = Vec::with_capacity(1000);
    let mut bytes = 0;
    for _ in 0..1000 {
        let start = std::time::Instant::now();
        world.advance(0.1).await;
        times.push(start.elapsed().as_nanos());
        bytes += rx.try_recv().unwrap().data.len();
    }
    times.sort_unstable();
    eprintln!(
        "HS150 active_ticks=1000 p50_us={:.2} p95_us={:.2} mean_bytes={}",
        times[500] as f64 / 1000.0,
        times[950] as f64 / 1000.0,
        bytes / 1000
    );
}

#[tokio::test]
async fn whole_turn_motor_ticks_still_confirm_motion_to_the_client() {
    for slug in ["heated-stirrer", "centrifuge", "vortex-mixer"] {
        let (world, mut rx, id) = setup(slug);
        match slug {
            "heated-stirrer" => {
                send(&world, &id, "load_vessel", json!({})).await;
                send(&world, &id, "power", json!({})).await;
            }
            "centrifuge" => {
                send(&world, &id, "lid", json!({})).await;
                for slot in [0, 2] {
                    send(&world, &id, "load_tube", json!({"slot":slot})).await;
                }
                send(&world, &id, "lid", json!({})).await;
            }
            _ => {
                send(&world, &id, "load_tube", json!({})).await;
            }
        }
        send(&world, &id, "set_speed", json!({"rpm":600})).await;
        send(
            &world,
            &id,
            if slug == "heated-stirrer" {
                "stir"
            } else {
                "start"
            },
            json!({}),
        )
        .await;
        for _ in 0..40 {
            world.advance(0.1).await;
        }
        let mut previous = last(&mut rx);
        for _ in 0..10 {
            world.advance(0.1).await;
            let message=rx.try_recv().expect("moving device must renew the 250ms client observation even when sampled angles repeat");
            let next: Value = serde_json::from_slice(&message.data).unwrap();
            assert_eq!(v(&next)["rpm"], 600.0);
            assert!(
                next["properties"]["model_state"]["time_s"]
                    .as_f64()
                    .unwrap()
                    > previous["properties"]["model_state"]["time_s"]
                        .as_f64()
                        .unwrap()
            );
            previous = next;
            assert!(rx.try_recv().is_err());
        }
    }
}
