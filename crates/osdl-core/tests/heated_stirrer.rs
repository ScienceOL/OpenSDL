use osdl_core::simulation::{
    stirrer::{StirrerConfig, StirrerState},
    thermal::ThermalPair,
    SampleInventory, SimulationSampleConfig,
};
use serde_json::json;
fn config() -> StirrerConfig {
    let config: osdl_core::config::SimulationConfig = serde_yaml::from_str(include_str!(
        "../../../assets/instruments/heated-stirrer/package/source/world.yaml"
    ))
    .unwrap();
    match config.devices[0].model.clone().unwrap() {
        osdl_core::simulation::InstrumentConfig::HeatedStirrer(config) => config,
        _ => panic!("stirrer fixture kind"),
    }
}
fn setup() -> (StirrerState, SampleInventory) {
    (
        StirrerState::new(config()),
        SampleInventory::new(&[SimulationSampleConfig {
            id: "water_vessel".into(),
            mass_kg: 0.25,
            position_m: [0.0, 0.0],
            radius_m: 0.04,
        }])
        .unwrap(),
    )
}
#[test]
fn stirrer_has_distinct_plate_liquid_probe_and_residual_heat() {
    let (mut state, mut samples) = setup();
    assert_eq!(
        state
            .apply("s", "heat", &json!({}), &mut samples)
            .unwrap_err(),
        "POWER_REQUIRED"
    );
    assert_eq!(
        state
            .apply("s", "probe", &json!({}), &mut samples)
            .unwrap_err(),
        "LOAD_VESSEL_FIRST"
    );
    for action in ["load_vessel", "probe", "power"] {
        state.apply("s", action, &json!({}), &mut samples).unwrap();
    }
    state
        .apply(
            "s",
            "set_temperature",
            &json!({"temperature_c":80}),
            &mut samples,
        )
        .unwrap();
    state
        .apply("s", "set_speed", &json!({"rpm":600}), &mut samples)
        .unwrap();
    for action in ["heat", "stir"] {
        state.apply("s", action, &json!({}), &mut samples).unwrap();
    }
    state.advance(0.1);
    assert_eq!(state.values()["rpm"], 30.0);
    for _ in 0..600 {
        state.advance(0.1);
    }
    let v = state.values();
    assert!(v["plate_c"].as_f64().unwrap() > v["liquid_c"].as_f64().unwrap());
    assert!(v["liquid_c"].as_f64().unwrap() > 22.0);
    assert_eq!(v["probe_c"], v["liquid_c"]);
    assert_eq!(v["rpm"], 600.0);
    assert_eq!(
        state
            .apply("s", "unload_vessel", &json!({}), &mut samples)
            .unwrap_err(),
        "WAIT_FOR_COOL_STOP"
    );
    state.apply("s", "stop", &json!({}), &mut samples).unwrap();
    let hot = state.thermal.plate_c;
    state.advance(1.0);
    assert!(state.thermal.plate_c < hot && state.thermal.plate_c > 22.0);
    assert_eq!(state.values()["rpm"], 300.0);
    state.advance(1.0);
    assert_eq!(state.values()["rpm"], 0.0);
    assert_eq!(
        state
            .apply("s", "set_speed", &json!({"rpm":2000}), &mut samples)
            .unwrap_err(),
        "PARAMETER_LIMIT"
    );
    assert_eq!(state.values()["target_rpm"], 600.0);
}
#[test]
fn heat_exchange_conserves_energy_and_is_step_convergent() {
    let mut a = ThermalPair::new(22.0);
    let mut b = a.clone();
    for _ in 0..1000 {
        a.advance(0.1, |_| 300.0, [360.0, 836.0], [6.0, 1.4, 0.7], 22.0, true);
    }
    for _ in 0..2000 {
        b.advance(0.05, |_| 300.0, [360.0, 836.0], [6.0, 1.4, 0.7], 22.0, true);
    }
    let energy = (a.plate_c - 22.0) * 360.0 + (a.liquid_c - 22.0) * 836.0;
    assert!((energy - (a.input_j - a.loss_j)).abs() < 1e-7);
    assert!((a.plate_c - b.plate_c).abs() < 0.01);
    assert!((a.liquid_c - b.liquid_c).abs() < 0.01);
}

#[test]
fn water_fixture_stays_in_its_nonboiling_domain_and_rejections_are_atomic() {
    let (mut state, mut samples) = setup();
    state
        .apply(
            "s",
            "set_temperature",
            &json!({"temperature_c":150}),
            &mut samples,
        )
        .unwrap();
    for action in ["power", "heat"] {
        state.apply("s", action, &json!({}), &mut samples).unwrap();
    }
    let before = state.values();
    assert_eq!(
        state
            .apply("s", "load_vessel", &json!({}), &mut samples)
            .unwrap_err(),
        "LOADED_WATER_LIMIT"
    );
    assert_eq!(before, state.values());
    state.apply("s", "stop", &json!({}), &mut samples).unwrap();
    state
        .apply("s", "load_vessel", &json!({}), &mut samples)
        .unwrap();
    assert_eq!(
        state
            .apply("s", "heat", &json!({}), &mut samples)
            .unwrap_err(),
        "LOADED_WATER_LIMIT"
    );
    let before = state.values();
    assert_eq!(
        state
            .apply(
                "s",
                "set_temperature",
                &json!({"temperature_c":100}),
                &mut samples
            )
            .unwrap_err(),
        "LOADED_WATER_LIMIT"
    );
    assert_eq!(before, state.values());
    state
        .apply(
            "s",
            "set_temperature",
            &json!({"temperature_c":95}),
            &mut samples,
        )
        .unwrap();
    state.apply("s", "heat", &json!({}), &mut samples).unwrap();
    let mut fine = state.clone();
    for _ in 0..1200 {
        state.advance(1.0);
    }
    for _ in 0..12000 {
        fine.advance(0.1);
    }
    assert!(state.thermal.plate_c <= 95.0 && state.thermal.liquid_c <= 95.0);
    assert!((state.thermal.plate_c - fine.thermal.plate_c).abs() < 0.01);
    assert!((state.thermal.liquid_c - fine.thermal.liquid_c).abs() < 0.01);
    // Hot empty plate cannot accept the water fixture either.
    let (mut empty, mut inventory) = setup();
    empty.thermal.plate_c = 120.0;
    assert_eq!(
        empty
            .apply("s", "load_vessel", &json!({}), &mut inventory)
            .unwrap_err(),
        "WAIT_FOR_COOL_STOP"
    );
}

#[test]
fn a_cold_stirrer_reports_coasting_until_the_motor_has_stopped() {
    let (mut state, mut samples) = setup();
    for action in ["load_vessel", "power"] {
        state.apply("s", action, &json!({}), &mut samples).unwrap();
    }
    state
        .apply("s", "set_speed", &json!({"rpm":600}), &mut samples)
        .unwrap();
    state.apply("s", "stir", &json!({}), &mut samples).unwrap();
    state.advance(2.0);
    state.apply("s", "stop", &json!({}), &mut samples).unwrap();
    assert_eq!(state.values()["status"], "decelerating");
    state.advance(1.0);
    assert_eq!(state.values()["status"], "decelerating");
    assert_eq!(state.values()["rpm"], 300.0);
    state.advance(1.0);
    assert_eq!(state.values()["status"], "ready");
    assert_eq!(state.values()["rpm"], 0.0);
}
