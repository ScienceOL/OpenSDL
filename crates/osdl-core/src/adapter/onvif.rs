//! ONVIF camera control adapter.
//!
//! Unlike `UniLabOsAdapter`, this adapter has no YAML registry — its device
//! types and actions are baked in. It also does not encode raw protocol
//! bytes: ONVIF is HTTP/SOAP, not a serial wire format. Instead, the
//! adapter encodes commands into a small JSON envelope that
//! [`crate::transport::onvif::OnvifTransport`] knows how to dispatch.
//!
//! Round-trip:
//! ```text
//!   adapter.encode_command  → JSON envelope bytes
//!                              ↓
//!   OnvifTransport.send     → SOAP / HTTP roundtrip
//!                              ↓
//!   OnvifTransport.rx_tx    → JSON response bytes
//!                              ↓
//!   adapter.decode_response → HashMap<String, Value> → DeviceStatus
//! ```
//!
//! Two device types are exposed:
//! * `onvif_camera` — `snapshot` action.
//! * `onvif_ptz`    — `ptz_move` / `ptz_stop` / `ptz_preset_goto` actions.
//!
//! In practice both are registered against the same physical camera; the
//! engine wires them up via [`crate::media::onvif_camera::OnvifControlConfig`].
//! `device_type` is what the engine asks for here, but `match_hardware` is
//! never called — cameras don't go through the hardware-id discovery path.

use crate::adapter::{DeviceMatch, ProtocolAdapter};
use crate::protocol::*;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const PLATFORM: &str = "onvif";

pub const DEVICE_TYPE_PTZ: &str = "onvif_ptz";
pub const DEVICE_TYPE_CAMERA: &str = "onvif_camera";

/// Combined PTZ+snapshot device type. Most lab cameras are physically one
/// unit, so the engine registers them under this single device_type rather
/// than two separate `Device`s sharing a transport.
pub const DEVICE_TYPE_COMBINED: &str = "onvif_camera_ptz";

pub struct OnvifAdapter;

impl OnvifAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Action schemas exposed by the combined PTZ + snapshot device type.
    /// Used by the engine when constructing the `Device` record so callers
    /// see actions in `lab device get`.
    pub fn combined_actions() -> Vec<ActionSchema> {
        vec![
            ptz_move_schema(),
            ptz_stop_schema(),
            ptz_preset_goto_schema(),
            snapshot_schema(),
        ]
    }
}

impl Default for OnvifAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolAdapter for OnvifAdapter {
    fn platform(&self) -> &str {
        PLATFORM
    }

    fn load_registry(&mut self, _path: &str) -> Result<(), String> {
        // Nothing to load — actions are built in.
        Ok(())
    }

    fn match_hardware(&self, _hardware_id: &str) -> Option<DeviceMatch> {
        // Cameras are registered explicitly from `media_sources`, not
        // discovered by hardware_id.
        None
    }

    fn encode_command(&self, device_type: &str, cmd: &DeviceCommand) -> Result<Vec<u8>, String> {
        let envelope = match device_type {
            DEVICE_TYPE_PTZ | DEVICE_TYPE_CAMERA | DEVICE_TYPE_COMBINED => {
                build_envelope(&cmd.action, &cmd.params)?
            }
            other => return Err(format!("onvif adapter: unknown device_type '{other}'")),
        };

        // Validate the action is supported by the device_type so a typo
        // surfaces as a clear error rather than a 500 from the camera.
        if !action_supported(device_type, &cmd.action) {
            return Err(format!(
                "onvif adapter: action '{}' not supported on device_type '{}'",
                cmd.action, device_type,
            ));
        }

        serde_json::to_vec(&envelope).map_err(|e| format!("onvif adapter: serialize envelope: {e}"))
    }

    fn decode_response(&self, _device_type: &str, bytes: &[u8]) -> Option<HashMap<String, Value>> {
        let envelope: Value = match serde_json::from_slice(bytes) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("onvif adapter: decode_response: invalid JSON: {e}");
                return None;
            }
        };

        let op = envelope.get("op").and_then(Value::as_str)?;
        let mut props: HashMap<String, Value> = HashMap::new();
        props.insert("last_op".into(), Value::String(op.to_string()));
        let ok = envelope.get("ok").and_then(Value::as_bool).unwrap_or(false);
        props.insert(format!("{op}_ok"), Value::Bool(ok));
        if let Some(err) = envelope.get("error").and_then(Value::as_str) {
            props.insert(format!("{op}_error"), Value::String(err.to_string()));
        }
        if let Some(data) = envelope.get("data") {
            // Snapshot path/url etc. land here. Flatten into properties so
            // a caller watching `device_status` events can read e.g.
            // `last_snapshot_url` without parsing nested JSON.
            if let Value::Object(map) = data {
                for (k, v) in map {
                    props.insert(format!("{op}_{k}"), v.clone());
                }
            }
        }
        Some(props)
    }
}

fn action_supported(device_type: &str, action: &str) -> bool {
    let ptz = matches!(action, "ptz_move" | "ptz_stop" | "ptz_preset_goto");
    let cam = matches!(action, "snapshot");
    match device_type {
        DEVICE_TYPE_PTZ => ptz,
        DEVICE_TYPE_CAMERA => cam,
        DEVICE_TYPE_COMBINED => ptz || cam,
        _ => false,
    }
}

/// Wrap (action, params) into the JSON envelope the transport understands.
/// We accept both ergonomic shorthand (`direction: "up"`) and the literal
/// PTZ vector (`pan`/`tilt`/`zoom`) so CLI users can type
/// `lab send cam1 ptz_move -p direction=up` without thinking in vectors.
fn build_envelope(action: &str, params: &Value) -> Result<Value, String> {
    let args = match action {
        "ptz_move" => normalize_ptz_move(params)?,
        "ptz_stop" => json!({}),
        "ptz_preset_goto" => normalize_preset(params)?,
        "snapshot" => json!({}),
        other => return Err(format!("onvif adapter: unknown action '{other}'")),
    };
    Ok(json!({ "op": action, "args": args }))
}

fn normalize_ptz_move(params: &Value) -> Result<Value, String> {
    // `direction` shorthand: up/down/left/right/in/out, with optional `speed`
    // (0..=1, default 0.5) and `duration_ms` (default 500). Falls back to
    // explicit pan/tilt/zoom vector when `direction` is absent. Either way
    // the on-wire ONVIF call is ContinuousMove + a delayed Stop; the
    // transport handles the timing.
    let speed = params
        .get("speed")
        .and_then(Value::as_f64)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let duration_ms = params
        .get("duration_ms")
        .and_then(Value::as_u64)
        .unwrap_or(500);

    if let Some(direction) = params.get("direction").and_then(Value::as_str) {
        let (pan, tilt, zoom) = match direction.to_ascii_lowercase().as_str() {
            "up" => (0.0, speed, 0.0),
            "down" => (0.0, -speed, 0.0),
            "left" => (-speed, 0.0, 0.0),
            "right" => (speed, 0.0, 0.0),
            "in" | "zoom_in" => (0.0, 0.0, speed),
            "out" | "zoom_out" => (0.0, 0.0, -speed),
            other => {
                return Err(format!(
                    "ptz_move: unknown direction '{other}' (expected up/down/left/right/in/out)",
                ))
            }
        };
        return Ok(json!({
            "pan": pan,
            "tilt": tilt,
            "zoom": zoom,
            "duration_ms": duration_ms,
        }));
    }

    let pan = params.get("pan").and_then(Value::as_f64).unwrap_or(0.0);
    let tilt = params.get("tilt").and_then(Value::as_f64).unwrap_or(0.0);
    let zoom = params.get("zoom").and_then(Value::as_f64).unwrap_or(0.0);
    if pan == 0.0 && tilt == 0.0 && zoom == 0.0 {
        return Err("ptz_move: requires either `direction` or non-zero pan/tilt/zoom".into());
    }
    Ok(json!({
        "pan": pan.clamp(-1.0, 1.0),
        "tilt": tilt.clamp(-1.0, 1.0),
        "zoom": zoom.clamp(-1.0, 1.0),
        "duration_ms": duration_ms,
    }))
}

fn normalize_preset(params: &Value) -> Result<Value, String> {
    let preset = params
        .get("preset")
        .and_then(Value::as_str)
        .ok_or("ptz_preset_goto: requires `preset` (string)")?;
    Ok(json!({ "preset": preset }))
}

fn ptz_move_schema() -> ActionSchema {
    ActionSchema {
        name: "ptz_move".into(),
        description: "Pan/tilt/zoom the camera. Shorthand: \
            `direction=up|down|left|right|in|out`. Explicit vector: \
            `pan`/`tilt`/`zoom` in [-1, 1]. Optional `speed` (0..1, \
            default 0.5) and `duration_ms` (default 500)."
            .into(),
        params: json!({
            "type": "object",
            "properties": {
                "direction": {"type": "string", "enum": ["up","down","left","right","in","out","zoom_in","zoom_out"]},
                "pan":  {"type": "number", "minimum": -1, "maximum": 1},
                "tilt": {"type": "number", "minimum": -1, "maximum": 1},
                "zoom": {"type": "number", "minimum": -1, "maximum": 1},
                "speed": {"type": "number", "minimum": 0, "maximum": 1},
                "duration_ms": {"type": "integer", "minimum": 0}
            }
        }),
    }
}

fn ptz_stop_schema() -> ActionSchema {
    ActionSchema {
        name: "ptz_stop".into(),
        description: "Stop any in-progress PTZ motion immediately.".into(),
        params: json!({"type": "object"}),
    }
}

fn ptz_preset_goto_schema() -> ActionSchema {
    ActionSchema {
        name: "ptz_preset_goto".into(),
        description: "Recall a stored PTZ preset by token / number.".into(),
        params: json!({
            "type": "object",
            "required": ["preset"],
            "properties": {"preset": {"type": "string"}}
        }),
    }
}

fn snapshot_schema() -> ActionSchema {
    ActionSchema {
        name: "snapshot".into(),
        description: "Capture a still JPEG from the camera. The captured \
            image is saved under `<data_dir>/snapshots/<cam_id>/<ts>.jpg` \
            and the path/URL surface as `snapshot_path` / `snapshot_url` \
            in the next `device_status` event."
            .into(),
        params: json!({"type": "object"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(action: &str, params: Value) -> DeviceCommand {
        DeviceCommand {
            command_id: "t".into(),
            device_id: "cam1".into(),
            action: action.into(),
            params,
        }
    }

    #[test]
    fn encodes_direction_shorthand() {
        let a = OnvifAdapter::new();
        let bytes = a
            .encode_command(
                DEVICE_TYPE_COMBINED,
                &cmd("ptz_move", json!({"direction": "up"})),
            )
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["op"], "ptz_move");
        assert_eq!(v["args"]["pan"], 0.0);
        assert_eq!(v["args"]["tilt"], 0.5);
    }

    #[test]
    fn rejects_unknown_direction() {
        let a = OnvifAdapter::new();
        let err = a
            .encode_command(
                DEVICE_TYPE_COMBINED,
                &cmd("ptz_move", json!({"direction": "north"})),
            )
            .unwrap_err();
        assert!(err.contains("unknown direction"));
    }

    #[test]
    fn explicit_vector_requires_motion() {
        let a = OnvifAdapter::new();
        let err = a
            .encode_command(DEVICE_TYPE_COMBINED, &cmd("ptz_move", json!({})))
            .unwrap_err();
        assert!(err.contains("non-zero"));
    }

    #[test]
    fn snapshot_action_only_on_camera_or_combined() {
        let a = OnvifAdapter::new();
        assert!(a
            .encode_command(DEVICE_TYPE_PTZ, &cmd("snapshot", json!({})))
            .is_err());
        assert!(a
            .encode_command(DEVICE_TYPE_CAMERA, &cmd("snapshot", json!({})))
            .is_ok());
        assert!(a
            .encode_command(DEVICE_TYPE_COMBINED, &cmd("snapshot", json!({})))
            .is_ok());
    }

    #[test]
    fn decode_flattens_data_into_props() {
        let a = OnvifAdapter::new();
        let resp = json!({
            "op": "snapshot",
            "ok": true,
            "data": {"path": "/tmp/x.jpg", "url": "file:///tmp/x.jpg"}
        });
        let bytes = serde_json::to_vec(&resp).unwrap();
        let props = a.decode_response(DEVICE_TYPE_COMBINED, &bytes).unwrap();
        assert_eq!(props["snapshot_ok"], Value::Bool(true));
        assert_eq!(props["snapshot_path"], Value::String("/tmp/x.jpg".into()));
        assert_eq!(props["last_op"], Value::String("snapshot".into()));
    }

    #[test]
    fn preset_goto_requires_preset_param() {
        let a = OnvifAdapter::new();
        assert!(a
            .encode_command(DEVICE_TYPE_COMBINED, &cmd("ptz_preset_goto", json!({})))
            .is_err());
        assert!(a
            .encode_command(
                DEVICE_TYPE_COMBINED,
                &cmd("ptz_preset_goto", json!({"preset": "1"}))
            )
            .is_ok());
    }
}
