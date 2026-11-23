//! Shared semantic checks for controller observations and GLB action controls.
//!
//! Cached-package admission calls this once for the initial state. `lab
//! validate` additionally feeds every action result and every advanced state
//! through the same checks, so a channel cannot pass solely because it was
//! valid at time zero. GLB structure is owned by `opensdl-assets`; this module
//! validates the contract between that presentation metadata and the runtime.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use opensdl_assets::model::{Control, JointKind, ModelDefinition};
use osdl_core::{config::SimulationDeviceConfig, protocol::ActionSchema};
use serde_json::{Map, Value};

/// Validates the initial observation and the complete simulator action catalog.
pub(crate) fn validate_initial_contract(
    device: &SimulationDeviceConfig,
    definition: &ModelDefinition,
    properties: &HashMap<String, Value>,
) -> Result<()> {
    for action in &device.actions {
        validate_schema_declaration(action)?;
    }
    validate_observation(device, definition, properties)
}

/// Cross-checks a live observation against all GLB bindings and controls.
///
/// Controller-owned observables not mentioned by the GLB are allowed. Every
/// declared presentation binding, however, must have a correctly typed finite
/// value, and every interactive control must still generate schema-valid
/// parameters for the current state.
pub(crate) fn validate_observation(
    device: &SimulationDeviceConfig,
    definition: &ModelDefinition,
    properties: &HashMap<String, Value>,
) -> Result<()> {
    let state = properties
        .get("model_state")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("instrument observation has no model_state object"))?;
    if state.get("schema_version").and_then(Value::as_u64) != Some(2) {
        bail!("instrument observation must use model_state schema_version 2");
    }
    if state.get("model_id").and_then(Value::as_str) != Some(definition.model_id.as_str()) {
        bail!("model_state model_id differs from the GLB declaration");
    }
    let values = state
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("model_state.values must be an object"))?;
    for joint in &definition.joints {
        let value = finite_number(values, &joint.state, "joint")?;
        if !matches!(joint.kind, JointKind::Continuous) && !(joint.min..=joint.max).contains(&value)
        {
            bail!(
                "joint '{}' state {value} exceeds declared range {}..={}",
                joint.id,
                joint.min,
                joint.max
            );
        }
        if let Some(velocity) = &joint.velocity_state {
            finite_number(values, velocity, "joint velocity")?;
        }
    }
    for binding in &definition.visibility {
        if !matches!(values.get(&binding.state), Some(Value::Bool(_))) {
            bail!(
                "visibility binding '{}' requires a boolean value",
                binding.state
            );
        }
    }
    for readout in &definition.readouts {
        let value = values
            .get(&readout.state)
            .ok_or_else(|| anyhow!("readout '{}' has no value", readout.state))?;
        if !value.is_null() && value.as_f64().is_none_or(|number| !number.is_finite()) {
            bail!(
                "readout '{}' requires null or a finite number",
                readout.state
            );
        }
    }
    for component in &definition.components {
        if let Some(scale) = &component.scale {
            let value = finite_number(values, &scale.state, "scale")?;
            if !(scale.min..=scale.max).contains(&value) {
                bail!(
                    "scale binding '{}' state {value} exceeds {}..={}",
                    component.id,
                    scale.min,
                    scale.max
                );
            }
        }
        let Some(control) = &component.control else {
            continue;
        };
        let action_name = match control {
            Control::Command { action, .. }
            | Control::Toggle { action, .. }
            | Control::Dial { action, .. } => action,
        };
        let action = device
            .actions
            .iter()
            .find(|action| &action.name == action_name)
            .ok_or_else(|| {
                anyhow!(
                    "GLB component '{}' has no action '{}'",
                    component.id,
                    action_name
                )
            })?;
        match control {
            Control::Command { params, .. } => {
                let params = params
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                validate_params(action, &Value::Object(params))?
            }
            Control::Toggle {
                params,
                parameter,
                state,
                open,
                closed,
                ..
            } => {
                finite_number(values, state, "toggle control")?;
                for value in [*open, *closed] {
                    let params = merge_parameter(params, parameter, Value::from(value));
                    validate_params(action, &params).with_context(|| {
                        format!("GLB toggle '{}' generated invalid parameters", component.id)
                    })?;
                }
            }
            Control::Dial {
                params,
                parameter,
                state,
                min,
                max,
                step,
                ..
            } => {
                let current = finite_number(values, state, "dial control")?;
                for next in [
                    (current + step).clamp(*min, *max),
                    (current - step).clamp(*min, *max),
                ] {
                    let params = merge_parameter(params, parameter, Value::from(next));
                    validate_params(action, &params).with_context(|| {
                        format!("GLB dial '{}' generated invalid parameters", component.id)
                    })?;
                }
            }
        }
    }
    Ok(())
}

fn merge_parameter(base: &HashMap<String, Value>, parameter: &str, value: Value) -> Value {
    let mut params: Map<String, Value> = base
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    params.insert(parameter.to_owned(), value);
    Value::Object(params)
}

fn finite_number(values: &Map<String, Value>, channel: &str, kind: &str) -> Result<f64> {
    values
        .get(channel)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| anyhow!("{kind} binding '{channel}' requires a finite number"))
}

/// Validates the closed JSON-Schema subset accepted for simulator actions.
///
/// Restricting the schema to objects containing scalar properties keeps UI
/// controls, CLI validation, and controller parameter handling on one bounded
/// contract instead of accepting schemas that no simulator consumer implements.
fn validate_schema_declaration(action: &ActionSchema) -> Result<()> {
    validate_identifier(&action.name, "action name")?;
    let schema = action
        .params
        .as_object()
        .ok_or_else(|| anyhow!("action '{}' params must be an object schema", action.name))?;
    for key in schema.keys() {
        if !matches!(
            key.as_str(),
            "type"
                | "properties"
                | "required"
                | "additionalProperties"
                | "title"
                | "description"
                | "$schema"
        ) {
            bail!(
                "action '{}' uses unsupported schema keyword '{key}'",
                action.name
            );
        }
    }
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        bail!("action '{}' params.type must be object", action.name);
    }
    if schema.get("additionalProperties").and_then(Value::as_bool) != Some(false) {
        bail!(
            "action '{}' must set additionalProperties to false",
            action.name
        );
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            anyhow!(
                "action '{}' params.properties must be an object",
                action.name
            )
        })?;
    let mut required = std::collections::HashSet::new();
    for value in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = value
            .as_str()
            .ok_or_else(|| anyhow!("action '{}' required entries must be strings", action.name))?;
        if !properties.contains_key(name) || !required.insert(name) {
            bail!(
                "action '{}' has an invalid or duplicate required '{name}'",
                action.name
            );
        }
    }
    for (name, schema) in properties {
        validate_property_schema(&action.name, name, schema)?;
    }
    Ok(())
}

fn validate_property_schema(action: &str, name: &str, schema: &Value) -> Result<()> {
    let schema = schema
        .as_object()
        .ok_or_else(|| anyhow!("action '{action}' property '{name}' schema must be an object"))?;
    for key in schema.keys() {
        if !matches!(
            key.as_str(),
            "type" | "enum" | "minimum" | "maximum" | "unit" | "description"
        ) {
            bail!("action '{action}' property '{name}' uses unsupported keyword '{key}'");
        }
    }
    let kind = schema
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("action '{action}' property '{name}' needs a scalar type"))?;
    if !matches!(kind, "number" | "integer" | "string" | "boolean") {
        bail!("action '{action}' property '{name}' has unsupported type '{kind}'");
    }
    if matches!(kind, "string" | "boolean")
        && (schema.contains_key("minimum") || schema.contains_key("maximum"))
    {
        bail!("action '{action}' property '{name}' cannot have a numeric range");
    }
    if let Some(values) = schema.get("enum") {
        let values = values
            .as_array()
            .filter(|values| !values.is_empty())
            .ok_or_else(|| anyhow!("action '{action}' property '{name}' enum must not be empty"))?;
        for value in values {
            validate_scalar_type(action, name, kind, value)?;
        }
    }
    for key in ["minimum", "maximum"] {
        if schema
            .get(key)
            .is_some_and(|value| value.as_f64().is_none_or(|number| !number.is_finite()))
        {
            bail!("action '{action}' property '{name}' {key} must be finite");
        }
    }
    if let (Some(minimum), Some(maximum)) = (
        schema.get("minimum").and_then(Value::as_f64),
        schema.get("maximum").and_then(Value::as_f64),
    ) {
        if minimum > maximum {
            bail!("action '{action}' property '{name}' has an inverted range");
        }
    }
    Ok(())
}

/// Validates one action invocation against the exact public action schema.
/// GLB controls and CLI conformance programs both pass through this boundary.
pub(crate) fn validate_action_params(action: &ActionSchema, params: &Value) -> Result<()> {
    validate_params(action, params)
}

fn validate_params(action: &ActionSchema, params: &Value) -> Result<()> {
    let schema = action
        .params
        .as_object()
        .expect("action schema was checked during admission");
    let properties = schema["properties"]
        .as_object()
        .expect("action properties were checked");
    let params = params
        .as_object()
        .ok_or_else(|| anyhow!("action '{}' params must be an object", action.name))?;
    for key in params.keys() {
        if !properties.contains_key(key) {
            bail!(
                "action '{}' received unknown parameter '{key}'",
                action.name
            );
        }
    }
    for value in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !params.contains_key(value) {
            bail!(
                "action '{}' is missing required parameter '{value}'",
                action.name
            );
        }
    }
    for (name, value) in params {
        let schema = &properties[name];
        let kind = schema["type"].as_str().expect("property type was checked");
        validate_scalar_type(&action.name, name, kind, value)?;
        if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
            let permitted = choices.iter().any(|choice| {
                match (choice.as_f64(), value.as_f64()) {
                    // JSON Schema compares numbers mathematically, so an enum
                    // containing 7 accepts both 7 and the dial-produced 7.0.
                    (Some(choice), Some(value)) => choice == value,
                    _ => choice == value,
                }
            });
            if !permitted {
                bail!(
                    "action '{}' parameter '{name}' is outside its enum",
                    action.name
                );
            }
        }
        if let Some(number) = value.as_f64() {
            if schema
                .get("minimum")
                .and_then(Value::as_f64)
                .is_some_and(|minimum| number < minimum)
                || schema
                    .get("maximum")
                    .and_then(Value::as_f64)
                    .is_some_and(|maximum| number > maximum)
            {
                bail!(
                    "action '{}' parameter '{name}' is outside its range",
                    action.name
                );
            }
        }
    }
    Ok(())
}

fn validate_scalar_type(action: &str, name: &str, kind: &str, value: &Value) -> Result<()> {
    let valid = match kind {
        "number" => value.as_f64().is_some_and(f64::is_finite),
        // JSON Schema defines integer mathematically: 10 and 10.0 are both
        // integers, including values generated by a JavaScript dial control.
        "integer" => value
            .as_f64()
            .is_some_and(|number| number.is_finite() && number.fract() == 0.0),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        _ => false,
    };
    if !valid {
        bail!("action {action:?} parameter {name:?} must be {kind}");
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    {
        bail!("{label} '{value}' must be a non-empty portable identifier");
    }
    Ok(())
}
