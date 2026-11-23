use std::sync::LazyLock;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::protocol::{ActionSchema, DeviceCommand};

use super::wire::Command;

mod payload;

static SCHEMAS: LazyLock<Vec<ActionSchema>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("actions.json")).expect("built-in SC9300 action schemas")
});

pub(super) fn schemas() -> Vec<ActionSchema> {
    SCHEMAS.clone()
}

#[derive(Clone, Copy, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub(super) enum Axis {
    Left = 1,
    Right = 2,
    Gripper = 3,
}

impl Axis {
    pub(super) const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Gripper];
}

pub(super) enum Operation {
    DeviceInfo,
    Status,
    Position(Axis),
    Telemetry,
    CheckLimit(Vec<u8>),
    Dispatch(Command, Vec<u8>),
}

pub(super) enum Request {
    Exchange(Operation),
    WaitIdle(Duration),
}

impl Request {
    pub(super) fn parse(command: &DeviceCommand) -> Result<Self, String> {
        let schema = SCHEMAS
            .iter()
            .find(|schema| schema.name == command.action)
            .ok_or("unsupported SC9300 action")?;
        let args = Arguments::validate(&command.params, &schema.params)?;
        use Command as C;
        use Operation::*;

        let operation = match command.action.as_str() {
            "device_info" => DeviceInfo,
            "status" => Status,
            "position" => Position(args.axis()),
            "telemetry" => Telemetry,
            "check_limit" => CheckLimit(payload::limit(&args)),
            "wait_idle" => {
                return Ok(Self::WaitIdle(Duration::from_secs(
                    args.integer("timeoutSeconds", 20),
                )));
            }
            "stop" => Dispatch(C::Stop, vec![]),
            "reset" => Dispatch(C::Reset, payload::reset()),
            "clear_error" => Dispatch(C::ClearError, vec![]),
            "set_motors" => Dispatch(C::Enable, vec![u8::from(args.enabled("enabled"))]),
            "move" => Dispatch(C::Move, payload::move_to(&args)),
            "jog" => Dispatch(C::Jog, payload::jog(&args)),
            "jaw" => Dispatch(C::Jog, payload::jaw(&args)),
            "pick_up_tips" => Dispatch(C::PickUpTips, payload::tips(&args, true)),
            "set_back_tips" => Dispatch(C::SetBackTips, payload::tips(&args, false)),
            "aspirate" => Dispatch(C::Aspirate, payload::aspirate(&args)?),
            "dispense" => Dispatch(C::Dispense, payload::dispense(&args)?),
            "homogenize" => Dispatch(C::Homogenize, payload::homogenize(&args)?),
            "clamp_plate" => Dispatch(C::Clamp, payload::clamp(&args)),
            "set_temperature" => Dispatch(C::Temperature, payload::temperature(&args)),
            "oscillate" => Dispatch(C::Oscillate, payload::oscillate(&args)),
            "temperature_and_oscillate" => Dispatch(
                C::TemperatureAndOscillate,
                payload::temperature_and_oscillate(&args),
            ),
            "magnetic_stand" => Dispatch(C::MagneticStand, payload::magnetic_stand(&args)),
            "set_light" => Dispatch(C::Cabinet, payload::cabinet(&args, 1, 0)),
            "set_ultraviolet" => Dispatch(C::Cabinet, payload::cabinet(&args, 3, 2)),
            "set_filter_fan" => Dispatch(C::Cabinet, payload::cabinet(&args, 10, 11)),
            _ => return Err("unsupported SC9300 action".into()),
        };
        Ok(Self::Exchange(operation))
    }
}

// The device advertises the same closed schema that admits command arguments.
// SC9300 parameters only use scalar types, enums and numeric bounds.
struct Arguments<'a>(Option<&'a Map<String, Value>>);

impl<'a> Arguments<'a> {
    fn validate(params: &'a Value, schema: &Value) -> Result<Self, String> {
        let object = match params {
            Value::Null => None,
            Value::Object(fields) => Some(fields),
            _ => return Err("SC9300 action arguments must be an object".into()),
        };
        let args = Self(object);
        for key in schema["required"].as_array().expect("required fields") {
            let key = key.as_str().expect("required field name");
            if args.get(key).is_none() {
                return Err(format!("missing argument '{key}'"));
            }
        }
        for (key, value) in object.into_iter().flatten() {
            let rule = schema["properties"]
                .get(key)
                .ok_or_else(|| format!("unexpected argument '{key}'"))?;
            if !valid_argument(value, rule) {
                return Err(format!("invalid argument '{key}'"));
            }
        }
        Ok(args)
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.0.and_then(|fields| fields.get(key))
    }

    fn axis(&self) -> Axis {
        match self.text("axis") {
            "left" => Axis::Left,
            "right" => Axis::Right,
            "gripper" => Axis::Gripper,
            _ => unreachable!("validated head"),
        }
    }

    fn text(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    fn enabled(&self, key: &str) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or_default()
    }

    fn integer(&self, key: &str, default: u64) -> u64 {
        self.get(key)
            .and_then(Value::as_f64)
            .map_or(default, |value| value as u64)
    }

    fn number(&self, key: &str, default: f32) -> f32 {
        self.get(key)
            .and_then(Value::as_f64)
            .map_or(default, |value| value as f32)
    }
}

fn valid_argument(value: &Value, rule: &Value) -> bool {
    let valid = match rule["type"].as_str() {
        Some("boolean") => value.is_boolean(),
        Some("string") => value.is_string(),
        Some(kind @ ("integer" | "number")) => value.as_f64().is_some_and(|number| {
            number.is_finite()
                && (kind == "number" || number.fract() == 0.0)
                && rule["minimum"].as_f64().is_none_or(|min| number >= min)
                && rule["exclusiveMinimum"]
                    .as_f64()
                    .is_none_or(|min| number > min)
                && rule["maximum"].as_f64().is_none_or(|max| number <= max)
        }),
        _ => false,
    };
    valid
        && rule["enum"].as_array().is_none_or(|choices| {
            choices.iter().any(|choice| {
                choice == value
                    || choice
                        .as_f64()
                        .zip(value.as_f64())
                        .is_some_and(|(a, b)| a == b)
            })
        })
}
