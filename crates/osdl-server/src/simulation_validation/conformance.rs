//! Deterministic happy-path programs for every built-in instrument controller.
//!
//! These are validation fixtures, not a second controller implementation. Each
//! program drives the production `InstrumentState` through its public actions,
//! advances the same production state transition, and is checked for complete
//! declared-action coverage by the parent module.

use osdl_core::{
    config::SimulationDeviceConfig,
    simulation::{InstrumentConfig, SimulationSampleConfig},
};
use serde_json::{json, Value};

pub(super) enum Step {
    Act(&'static str, Value),
    Advance(f64),
}

fn act(action: &'static str) -> Step {
    Step::Act(action, json!({}))
}

fn with(action: &'static str, params: Value) -> Step {
    Step::Act(action, params)
}

fn advance(seconds: f64) -> Step {
    Step::Advance(seconds)
}

fn motor_stop(max_rpm: f64, slew_rpm_s: f64) -> Step {
    advance(max_rpm / slew_rpm_s + 1.0)
}

/// Builds a normal-operation sequence from immutable model configuration.
/// Numeric choices remain inside each model's validated limits and therefore
/// do not duplicate magic values from the authored world YAML.
pub(super) fn program(
    device: &SimulationDeviceConfig,
    samples: &[SimulationSampleConfig],
) -> Vec<Step> {
    let mut steps = vec![act("reset")];
    let model = device
        .model
        .as_ref()
        .expect("dynamic programs are only built for model-backed instruments");
    match model {
        InstrumentConfig::AnalyticalBalance(config) => {
            let open = json!({"door":"left","opening_m":config.door_travel_m});
            let close = json!({"door":"left","opening_m":0.0});
            let sample = &samples
                .first()
                .expect("analytical balance requires a sample fixture")
                .id;
            steps.extend([
                act("power"),
                with("set_door", open),
                with("load_sample", json!({"sample_id":sample})),
                with("set_door", close),
                advance(config.settling_s + 0.5),
                act("tare"),
                with(
                    "set_door",
                    json!({"door":"left","opening_m":config.door_travel_m}),
                ),
                with("unload_sample", json!({"sample_id":sample})),
                with("set_door", json!({"door":"left","opening_m":0.0})),
                advance(config.settling_s + 0.5),
                act("zero"),
            ]);
        }
        InstrumentConfig::HeatedStirrer(config) => steps.extend([
            act("power"),
            act("load_vessel"),
            act("probe"),
            with("set_temperature", json!({"temperature_c":config.ambient_c})),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            act("heat"),
            act("stir"),
            advance(0.1),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("probe"),
            act("unload_vessel"),
        ]),
        InstrumentConfig::WaterBath(config)
        | InstrumentConfig::DryingOven(config)
        | InstrumentConfig::DryBlockHeater(config) => steps.extend([
            act("lid"),
            act("load_sample"),
            with("set_temperature", json!({"temperature_c":config.ambient_c})),
            act("lid"),
            act("start"),
            advance(0.1),
            act("stop"),
            act("lid"),
            act("unload_sample"),
        ]),
        InstrumentConfig::LaboratoryRefrigerator(config) => steps.extend([
            act("lid"),
            act("load_sample"),
            with("set_temperature", json!({"temperature_c":config.max_c})),
            act("lid"),
            act("start"),
            advance(0.01),
            act("stop"),
            act("lid"),
            act("unload_sample"),
        ]),
        InstrumentConfig::Centrifuge(config) => steps.extend([
            act("lid"),
            with("load_tube", json!({"slot":0})),
            with("load_tube", json!({"slot":2})),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            act("lid"),
            act("start"),
            advance(0.1),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("lid"),
            with("unload_tube", json!({"slot":0})),
            with("unload_tube", json!({"slot":2})),
        ]),
        InstrumentConfig::VortexMixer(config) => steps.extend([
            act("load_tube"),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            act("start"),
            advance(0.1),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("unload_tube"),
        ]),
        InstrumentConfig::OrbitalShaker(config) => steps.extend([
            act("load_flask"),
            act("clamp"),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            act("start"),
            advance(0.1),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("clamp"),
            act("unload_flask"),
        ]),
        InstrumentConfig::TubeRoller(config) => steps.extend([
            act("load_tube"),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            act("start"),
            advance(0.1),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("unload_tube"),
        ]),
        InstrumentConfig::OverheadStirrer(config) => steps.extend([
            act("load_vessel"),
            act("lift"),
            with("set_height", json!({"height_m":config.immersion_max_m})),
            act("clamp"),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            act("start"),
            advance(0.1),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("clamp"),
            with("set_height", json!({"height_m":config.lift_m})),
            act("unload_vessel"),
        ]),
        InstrumentConfig::UltrasonicCleaner(config) => steps.extend([
            act("lid"),
            with("set_fill", json!({"fraction":config.min_fill})),
            with("set_temperature", json!({"temperature_c":config.ambient_c})),
            with(
                "set_duration",
                json!({"seconds":1.0_f64.min(config.max_duration_s)}),
            ),
            act("load_basket"),
            act("lid"),
            act("start"),
            advance(1.0),
            act("lid"),
            act("unload_basket"),
            act("drain"),
            act("stop"),
        ]),
        InstrumentConfig::PeristalticPump(config) => steps.extend([
            act("lid"),
            act("load_tubing"),
            act("lid"),
            act("prime"),
            with(
                "set_flow",
                json!({"flow_ml_min":config.max_rpm * config.displacement_ml_rev / 100.0}),
            ),
            act("start"),
            advance(0.01),
            act("stop"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            act("lid"),
            act("unload_tubing"),
        ]),
        InstrumentConfig::ConductivityMeter(config) => steps.extend([
            act("power"),
            with("set_solution", json!({"solution":"standard"})),
            with("set_temperature", json!({"temperature_c":25.0})),
            act("load_sample"),
            act("probe"),
            advance(config.response_s * 10.0),
            act("calibrate"),
            act("probe"),
            with("set_solution", json!({"solution":"sample"})),
            act("probe"),
            advance(config.response_s),
            act("probe"),
            act("unload_sample"),
        ]),
        InstrumentConfig::UvVisSpectrophotometer(config) => steps.extend([
            act("power"),
            with("set_wavelength", json!({"wavelength_nm":config.peak_nm})),
            advance(config.warmup_s),
            act("lid"),
            with("load_cuvette", json!({"kind":"blank"})),
            act("lid"),
            act("blank"),
            act("lid"),
            act("unload_cuvette"),
            with("load_cuvette", json!({"kind":"sample"})),
            act("lid"),
            act("measure"),
            act("lid"),
            act("unload_cuvette"),
        ]),
        InstrumentConfig::ThermalCycler(_) => steps.extend([
            with("set_cycles", json!({"cycles":1})),
            act("lid"),
            act("load_tubes"),
            act("lid"),
            act("latch"),
            act("start"),
            advance(0.1),
            act("stop"),
            // Drive the actual interrupted-program cooling state to completion
            // before unlatching or unloading; no precondition is bypassed.
            advance(20_000.0),
            act("latch"),
            act("lid"),
            act("unload_tubes"),
        ]),
        InstrumentConfig::VacuumFiltration(_) => steps.extend([
            act("assemble"),
            act("install_filter"),
            act("load_slurry"),
            act("seal"),
            act("start"),
            advance(0.1),
            act("stop"),
            act("vent"),
            advance(1.0),
            act("seal"),
            act("clear_slurry"),
            act("remove_filter"),
            act("assemble"),
            act("empty_receiver"),
        ]),
        InstrumentConfig::RotaryEvaporator(config) => steps.extend([
            act("load_flask"),
            act("clamp"),
            with("set_height", json!({"height_m":config.immersion_max_m})),
            with("set_speed", json!({"rpm":config.max_rpm / 10.0})),
            with("set_temperature", json!({"temperature_c":config.ambient_c})),
            act("start"),
            act("heat"),
            act("start_vacuum"),
            advance(0.01),
            act("stop_vacuum"),
            act("stop"),
            act("vent"),
            motor_stop(config.max_rpm, config.slew_rpm_s),
            with("set_height", json!({"height_m":config.lift_m})),
            act("lift"),
            with("set_height", json!({"height_m":config.lift_m})),
            act("clamp"),
            act("unload_flask"),
        ]),
        InstrumentConfig::Pipette(config) => steps.extend([
            act("tip"),
            with("set_volume", json!({"volume_ul":config.min_ul})),
            with("move_to", json!({"position":"source"})),
            act("aspirate"),
            with("move_to", json!({"position":"receiver"})),
            act("dispense"),
            with("move_to", json!({"position":"source"})),
            act("stroke"),
            with("move_to", json!({"position":"receiver"})),
            act("stroke"),
        ]),
        InstrumentConfig::SyringePump(config) => steps.extend([
            with(
                "set_flow",
                json!({"flow_ml_min":config.max_flow_ml_min / 100.0}),
            ),
            act("start"),
            advance(0.01),
            act("stop"),
            act("refill"),
        ]),
        InstrumentConfig::VacuumPump(_) => {
            steps.extend([act("start"), advance(0.1), act("stop"), act("vent")])
        }
        InstrumentConfig::PHMeter(config) => steps.extend([
            act("power"),
            with("set_buffer", json!({"ph":7.0})),
            act("probe"),
            advance(config.response_s * 4.0),
            act("calibrate"),
            act("probe"),
        ]),
    }
    steps
}
