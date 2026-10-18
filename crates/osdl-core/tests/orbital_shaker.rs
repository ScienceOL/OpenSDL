use osdl_core::{
    config::SimulationConfig,
    simulation::{orbital::OrbitalState, InstrumentConfig, SampleInventory},
};
use serde_json::{json, Value};

fn fixture() -> (OrbitalState, SampleInventory) {
    let config: SimulationConfig = serde_yaml::from_str(include_str!(
        "../../../assets/instruments/orbital-shaker/package/source/world.yaml"
    ))
    .unwrap();
    let InstrumentConfig::OrbitalShaker(model) = &config.devices[0].model.as_ref().unwrap() else {
        panic!("authored world must select the orbital model");
    };
    model.validate().unwrap();
    (
        OrbitalState::new(model.clone()),
        SampleInventory::new(&config.samples).unwrap(),
    )
}

fn apply(state: &mut OrbitalState, samples: &mut SampleInventory, action: &str, params: Value) {
    state.apply("shaker", action, &params, samples).unwrap();
}

#[test]
fn a_flask_must_be_secured_and_stay_locked_until_the_platform_stops() {
    let (mut state, mut samples) = fixture();
    apply(&mut state, &mut samples, "set_speed", json!({"rpm": 240}));
    for (required_action, failure) in [
        ("load_flask", "LOAD_SAMPLE_FIRST"),
        ("clamp", "SECURE_SAMPLE_FIRST"),
    ] {
        let before = state.values();
        assert_eq!(
            state
                .apply("shaker", "start", &json!({}), &mut samples)
                .unwrap_err(),
            failure
        );
        assert_eq!(
            state.values(),
            before,
            "a rejected command cannot mutate state"
        );
        apply(&mut state, &mut samples, required_action, json!({}));
    }
    apply(&mut state, &mut samples, "start", json!({}));
    // Even before the first motion tick, a running motor owns the fixture.
    for action in ["clamp", "unload_flask", "reset"] {
        assert_eq!(
            state
                .apply("shaker", action, &json!({}), &mut samples)
                .unwrap_err(),
            "MOTOR_MOVING"
        );
    }
    state.advance(2.0);
    assert_eq!(state.values()["rpm"], 240.0);
    assert_eq!(
        state.values()["platform_rad"],
        -state.values()["orbit_rad"].as_f64().unwrap()
    );
    apply(&mut state, &mut samples, "stop", json!({}));
    state.advance(1.0);
    assert_eq!(state.values()["status"], "decelerating");
    assert_eq!(
        state
            .apply("shaker", "clamp", &json!({}), &mut samples)
            .unwrap_err(),
        "MOTOR_MOVING"
    );
    state.advance(1.0);
    assert_eq!(state.values()["rpm"], 0.0);
    assert_eq!(
        state
            .apply("shaker", "unload_flask", &json!({}), &mut samples)
            .unwrap_err(),
        "UNCLAMP_SAMPLE_FIRST"
    );
    apply(&mut state, &mut samples, "clamp", json!({}));
    apply(&mut state, &mut samples, "unload_flask", json!({}));
    assert_eq!(state.values()["flask_visible"], false);
    samples
        .claim("orbital_flask", "another-device", 0.05)
        .unwrap();
}

#[test]
fn ramp_angle_is_step_partition_invariant_and_target_limits_are_atomic() {
    let (mut one, mut samples) = fixture();
    for action in ["load_flask", "clamp"] {
        apply(&mut one, &mut samples, action, json!({}));
    }
    apply(&mut one, &mut samples, "set_speed", json!({"rpm": 300}));
    apply(&mut one, &mut samples, "start", json!({}));
    let before = one.values();
    assert!(one
        .apply("shaker", "set_speed", &json!({"rpm": 501}), &mut samples)
        .is_err());
    assert_eq!(one.values(), before);
    let mut many = one.clone();
    one.advance(5.0);
    for _ in 0..50 {
        many.advance(0.1);
    }
    for key in [
        "rpm",
        "orbit_rad",
        "platform_rad",
        "orbit_rad_s",
        "platform_rad_s",
    ] {
        assert!(
            (one.values()[key].as_f64().unwrap() - many.values()[key].as_f64().unwrap()).abs()
                < 1e-10,
            "{key}"
        );
    }
    apply(&mut one, &mut samples, "stop", json!({}));
    many.apply("shaker", "stop", &json!({}), &mut samples)
        .unwrap();
    one.advance(5.0);
    for _ in 0..50 {
        many.advance(0.1);
    }
    assert!(
        (one.values()["orbit_rad"].as_f64().unwrap()
            - many.values()["orbit_rad"].as_f64().unwrap())
        .abs()
            < 1e-10
    );
}
