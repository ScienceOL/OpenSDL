mod support;
use serde_json::json;
use support::instruments::{last, send, setup, v};

#[tokio::test]
async fn refrigeration_retains_cold_and_pauses_cooling_when_the_door_is_open() {
    let (world, mut rx, id) = setup("laboratory-refrigerator");
    assert_eq!(
        world
            .apply(&id, "load_sample", &json!({}))
            .await
            .unwrap_err(),
        "OPEN_DOOR_FIRST"
    );
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "load_sample", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "set_temperature", json!({"temperature_c":4})).await;
    send(&world, &id, "start", json!({})).await;
    world.advance(300.0).await;
    let early = last(&mut rx);
    assert!(v(&early)["temperature_c"].as_f64().unwrap() < 22.0);
    assert!(v(&early)["sample_c"].as_f64().unwrap() > v(&early)["temperature_c"].as_f64().unwrap());
    world.advance(3300.0).await;
    let chilled = last(&mut rx);
    let temperature = v(&chilled)["temperature_c"].as_f64().unwrap();
    assert!((4.0..8.0).contains(&temperature));
    let opened = send(&world, &id, "lid", json!({})).await;
    assert_eq!(v(&opened)["cooling"], false);
    assert_eq!(v(&opened)["running"], true);
    world.advance(120.0).await;
    let warming = last(&mut rx);
    assert!(v(&warming)["temperature_c"].as_f64().unwrap() > temperature);
    send(&world, &id, "lid", json!({})).await;
    world.advance(120.0).await;
    let recovered = last(&mut rx);
    assert!(
        v(&recovered)["temperature_c"].as_f64().unwrap()
            < v(&warming)["temperature_c"].as_f64().unwrap()
    );
    let stop = send(&world, &id, "stop", json!({})).await;
    assert_eq!(v(&stop)["temperature_c"], v(&recovered)["temperature_c"]);
    assert!(v(&stop)["temperature_c"].as_f64().unwrap() < 22.0);
    assert_eq!(
        world.apply(&id, "reset", &json!({})).await.unwrap_err(),
        "WAIT_FOR_AMBIENT_STOP"
    );
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_sample", json!({})).await;
    let loaded = send(&world, &id, "load_sample", json!({})).await;
    assert_eq!(v(&loaded)["sample_c"], 22.0);
}

#[tokio::test]
async fn conductivity_calibration_uses_a_settled_standard_and_compensates_temperature() {
    let (world, mut rx, id) = setup("conductivity-meter");
    send(&world, &id, "power", json!({})).await;
    assert_eq!(
        world.apply(&id, "probe", &json!({})).await.unwrap_err(),
        "LOAD_SAMPLE_FIRST"
    );
    send(&world, &id, "set_solution", json!({"solution":"standard"})).await;
    send(&world, &id, "set_temperature", json!({"temperature_c":35})).await;
    send(&world, &id, "load_sample", json!({})).await;
    send(&world, &id, "probe", json!({})).await;
    assert_eq!(
        world.apply(&id, "calibrate", &json!({})).await.unwrap_err(),
        "WAIT_FOR_CONDUCTIVITY_STANDARD"
    );
    assert_eq!(
        world
            .apply(&id, "unload_sample", &json!({}))
            .await
            .unwrap_err(),
        "RAISE_PROBE_FIRST"
    );
    world.advance(30.0).await;
    let before = last(&mut rx);
    assert_eq!(v(&before)["stable"], true);
    assert!(v(&before)["conductivity_25_us_cm"].as_f64().unwrap() > 1440.0);
    let calibrated = send(&world, &id, "calibrate", json!({})).await;
    assert!((v(&calibrated)["conductivity_25_us_cm"].as_f64().unwrap() - 1413.0).abs() < 1e-8);
    assert!((v(&calibrated)["conductivity_us_cm"].as_f64().unwrap() - 1695.6).abs() < 1e-8);
    send(&world, &id, "probe", json!({})).await;
    send(&world, &id, "set_solution", json!({"solution":"sample"})).await;
    send(&world, &id, "set_temperature", json!({"temperature_c":15})).await;
    send(&world, &id, "probe", json!({})).await;
    world.advance(30.0).await;
    let sample = last(&mut rx);
    assert!((v(&sample)["conductivity_25_us_cm"].as_f64().unwrap() - 800.0).abs() < 0.01);
    assert!((v(&sample)["conductivity_us_cm"].as_f64().unwrap() - 640.0).abs() < 0.01);
    let dry = send(&world, &id, "probe", json!({})).await;
    assert!(v(&dry)["conductivity_25_us_cm"].is_null());
    assert_eq!(v(&dry)["status"], "dry_probe");
}

#[tokio::test]
async fn spectrophotometer_requires_a_wavelength_specific_blank_and_a_closed_optical_path() {
    let (world, mut rx, id) = setup("uv-vis-spectrophotometer");
    send(&world, &id, "power", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "load_cuvette", json!({"kind":"blank"})).await;
    send(&world, &id, "lid", json!({})).await;
    assert_eq!(
        world.apply(&id, "blank", &json!({})).await.unwrap_err(),
        "WAIT_FOR_LAMP"
    );
    world.advance(3.0).await;
    last(&mut rx);
    send(&world, &id, "blank", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_cuvette", json!({})).await;
    send(&world, &id, "load_cuvette", json!({"kind":"sample"})).await;
    assert_eq!(
        world.apply(&id, "measure", &json!({})).await.unwrap_err(),
        "CLOSE_LID_FIRST"
    );
    send(&world, &id, "lid", json!({})).await;
    let peak = send(&world, &id, "measure", json!({})).await;
    assert!((v(&peak)["absorbance"].as_f64().unwrap() - 1.2).abs() < 1e-10);
    assert!((v(&peak)["transmittance_pct"].as_f64().unwrap() - 6.3095734448).abs() < 1e-8);
    let changed = send(&world, &id, "set_wavelength", json!({"wavelength_nm":700})).await;
    assert!(v(&changed)["absorbance"].is_null());
    assert_eq!(
        world.apply(&id, "measure", &json!({})).await.unwrap_err(),
        "BLANK_FIRST"
    );
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_cuvette", json!({})).await;
    send(&world, &id, "load_cuvette", json!({"kind":"blank"})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "blank", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_cuvette", json!({})).await;
    send(&world, &id, "load_cuvette", json!({"kind":"sample"})).await;
    send(&world, &id, "lid", json!({})).await;
    let tail = send(&world, &id, "measure", json!({})).await;
    assert!((v(&tail)["absorbance"].as_f64().unwrap() - 0.0503857820).abs() < 1e-8);
    let opened = send(&world, &id, "lid", json!({})).await;
    assert!(v(&opened)["absorbance"].is_null());
}

#[tokio::test]
async fn cycler_qualifies_holds_by_sample_and_lid_temperature_and_cools_before_unlocking() {
    let (world, mut rx, id) = setup("thermal-cycler");
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "load_tubes", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    assert_eq!(
        world.apply(&id, "start", &json!({})).await.unwrap_err(),
        "LATCH_FIRST"
    );
    send(&world, &id, "latch", json!({})).await;
    send(&world, &id, "set_cycles", json!({"cycles":1})).await;
    send(&world, &id, "start", json!({})).await;
    world.advance(60.0).await;
    let ramp = last(&mut rx);
    assert_eq!(v(&ramp)["hold_remaining_s"], 5.0);
    assert_eq!(v(&ramp)["stage"], 1);
    assert!(v(&ramp)["sample_c"].as_f64().unwrap() < v(&ramp)["temperature_c"].as_f64().unwrap());
    assert_eq!(
        world.apply(&id, "latch", &json!({})).await.unwrap_err(),
        "WAIT_FOR_COOL_STOP"
    );
    world.advance(1940.0).await;
    let complete = last(&mut rx);
    assert_eq!(v(&complete)["status"], "complete");
    assert_eq!(v(&complete)["aborted"], false);
    assert_eq!(v(&complete)["running"], false);
    assert!(v(&complete)["sample_c"].as_f64().unwrap() < 45.0);
    assert!(v(&complete)["lid_temperature_c"].as_f64().unwrap() < 45.0);
    send(&world, &id, "latch", json!({})).await;
    send(&world, &id, "lid", json!({})).await;
    send(&world, &id, "unload_tubes", json!({})).await;
}

#[tokio::test]
async fn aborting_a_cycler_cannot_erase_heat_or_report_successful_program_completion() {
    let (world, mut rx, id) = setup("thermal-cycler");
    for a in ["lid", "load_tubes", "lid", "latch", "start"] {
        send(&world, &id, a, json!({})).await;
    }
    world.advance(100.0).await;
    let hot = last(&mut rx);
    let stopped = send(&world, &id, "stop", json!({})).await;
    assert_eq!(v(&stopped)["temperature_c"], v(&hot)["temperature_c"]);
    assert_eq!(v(&stopped)["status"], "cooling");
    assert_eq!(v(&stopped)["aborted"], true);
    assert_eq!(
        world.apply(&id, "reset", &json!({})).await.unwrap_err(),
        "WAIT_FOR_COOL_STOP"
    );
    world.advance(2000.0).await;
    let aborted = last(&mut rx);
    assert_eq!(v(&aborted)["status"], "aborted");
    assert_eq!(v(&aborted)["running"], false);
}
