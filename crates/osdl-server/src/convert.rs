//! Conversions between osdl-core internal types and osdl-proto wire types.
//!
//! Internal types own their semantics; wire types own their stability. We
//! translate at the boundary so refactors of one don't ripple into the
//! other. All conversions here are lossy in only one direction: serde_json
//! → prost-types::Struct can fail when JSON contains a key that isn't a
//! string (impossible by construction), so we treat it as infallible.

use osdl_core::media::{MediaEndpoint as CoreMediaEndpoint, Protocol as CoreProtocol};
use osdl_core::protocol::{
    ActionSchema as CoreActionSchema, CommandResult as CoreCommandResult,
    CommandStatus as CoreCommandStatus, Device as CoreDevice, DeviceStatus as CoreDeviceStatus,
    Node as CoreNode,
};
use osdl_core::{OsdlEvent as CoreEvent, OsdlStatus as CoreStatus};
use osdl_proto::v1 as pb;
use prost_types::{value::Kind as PbKind, ListValue, NullValue, Struct as PbStruct, Value as PbValue};
use std::collections::HashMap;
use std::time::SystemTime;

// === Devices / Nodes ===

pub fn node_to_pb(n: &CoreNode) -> pb::Node {
    pb::Node {
        node_id: n.node_id.clone(),
        hardware_id: n.hardware_id.clone(),
        baud_rate: n.baud_rate,
        online: n.online,
        device_id: n.device_id.clone(),
    }
}

pub fn action_to_pb(a: &CoreActionSchema) -> pb::ActionSchema {
    pb::ActionSchema {
        name: a.name.clone(),
        description: a.description.clone(),
        params: Some(json_to_struct(&a.params)),
    }
}

pub fn device_to_pb(d: &CoreDevice) -> pb::Device {
    let properties = hashmap_to_struct(&d.properties);
    pb::Device {
        id: d.id.clone(),
        transport_id: d.transport_id.clone(),
        device_type: d.device_type.clone(),
        adapter: d.adapter.clone(),
        description: d.description.clone(),
        online: d.online,
        properties: Some(properties),
        actions: d.actions.iter().map(action_to_pb).collect(),
        role: d.role.clone(),
    }
}

// === Engine status ===

pub fn status_to_pb(s: &CoreStatus) -> pb::EngineStatus {
    use pb::engine_status::State;
    match s {
        CoreStatus::Disconnected => pb::EngineStatus {
            state: State::Disconnected as i32,
            broker: String::new(),
            node_count: 0,
            device_count: 0,
            error_message: String::new(),
        },
        CoreStatus::Connecting => pb::EngineStatus {
            state: State::Connecting as i32,
            broker: String::new(),
            node_count: 0,
            device_count: 0,
            error_message: String::new(),
        },
        CoreStatus::Connected {
            broker,
            node_count,
            device_count,
        } => pb::EngineStatus {
            state: State::Connected as i32,
            broker: broker.clone(),
            node_count: *node_count as u32,
            device_count: *device_count as u32,
            error_message: String::new(),
        },
        CoreStatus::Error { message } => pb::EngineStatus {
            state: State::Error as i32,
            broker: String::new(),
            node_count: 0,
            device_count: 0,
            error_message: message.clone(),
        },
    }
}

// === Commands ===

pub fn cmd_status_to_pb(s: &CoreCommandStatus) -> i32 {
    use pb::command_result::Status;
    match s {
        CoreCommandStatus::Pending => Status::Pending as i32,
        CoreCommandStatus::Running => Status::Running as i32,
        CoreCommandStatus::Succeeded => Status::Succeeded as i32,
        CoreCommandStatus::Failed => Status::Failed as i32,
        CoreCommandStatus::Cancelled => Status::Cancelled as i32,
    }
}

pub fn command_result_to_pb(r: &CoreCommandResult) -> pb::CommandResult {
    pb::CommandResult {
        command_id: r.command_id.clone(),
        device_id: r.device_id.clone(),
        status: cmd_status_to_pb(&r.status),
        message: r.message.clone(),
        data: r.data.as_ref().map(json_to_struct),
    }
}

// === Events ===

pub fn event_to_pb(ev: &CoreEvent) -> pb::Event {
    use pb::event::Kind;
    let kind = match ev {
        CoreEvent::DeviceOnline(d) => Kind::DeviceOnline(pb::DeviceOnlineEvent {
            device: Some(device_to_pb(d)),
        }),
        CoreEvent::DeviceOffline { device_id } => Kind::DeviceOffline(pb::DeviceOfflineEvent {
            device_id: device_id.clone(),
        }),
        CoreEvent::DeviceStatus(s) => Kind::DeviceStatus(device_status_to_pb(s)),
        CoreEvent::CommandResult(r) => Kind::CommandResult(pb::CommandResultEvent {
            result: Some(command_result_to_pb(r)),
        }),
        CoreEvent::UnknownNode {
            node_id,
            hardware_id,
        } => Kind::UnknownNode(pb::UnknownNodeEvent {
            node_id: node_id.clone(),
            hardware_id: hardware_id.clone(),
        }),
        CoreEvent::MediaSourceOnline {
            id,
            description,
            endpoints,
        } => Kind::MediaSourceOnline(pb::MediaSourceOnlineEvent {
            id: id.clone(),
            description: description.clone(),
            endpoints: endpoints.iter().map(media_endpoint_to_pb).collect(),
        }),
        CoreEvent::MediaGatewayDown { reason } => {
            Kind::MediaGatewayDown(pb::MediaGatewayDownEvent {
                reason: reason.clone(),
            })
        }
    };
    pb::Event {
        timestamp: Some(now_ts()),
        kind: Some(kind),
    }
}

pub fn lagged_event(dropped: u64) -> pb::Event {
    pb::Event {
        timestamp: Some(now_ts()),
        kind: Some(pb::event::Kind::Lagged(pb::LaggedEvent { dropped })),
    }
}

fn device_status_to_pb(s: &CoreDeviceStatus) -> pb::DeviceStatusEvent {
    pb::DeviceStatusEvent {
        device_id: s.device_id.clone(),
        timestamp: Some(unix_ms_to_ts(s.timestamp)),
        properties: Some(hashmap_to_struct(&s.properties)),
    }
}

fn media_endpoint_to_pb(e: &CoreMediaEndpoint) -> pb::MediaEndpoint {
    pb::MediaEndpoint {
        protocol: protocol_str(e.protocol).to_string(),
        location: format!("{:?}", e.location).to_lowercase(),
        url: e.url.clone(),
    }
}

fn protocol_str(p: CoreProtocol) -> &'static str {
    match p {
        CoreProtocol::Rtsp => "rtsp",
        CoreProtocol::Hls => "hls",
        CoreProtocol::Webrtc => "webrtc",
        CoreProtocol::Rtmp => "rtmp",
        CoreProtocol::Flv => "flv",
    }
}

// === Time ===

pub fn now_ts() -> prost_types::Timestamp {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    prost_types::Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

pub fn unix_ms_to_ts(ms: i64) -> prost_types::Timestamp {
    let secs = ms.div_euclid(1000);
    let nanos = (ms.rem_euclid(1000) * 1_000_000) as i32;
    prost_types::Timestamp { seconds: secs, nanos }
}

// === serde_json ↔ prost_types::Struct ===
//
// We use Struct rather than serde_json directly on the wire so prost-style
// codegen handles the bytes; the `properties` and `params` fields are
// dynamic and protobuf has a built-in shape for that.

pub fn struct_to_json(s: &PbStruct) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(s.fields.len());
    for (k, v) in &s.fields {
        map.insert(k.clone(), value_to_json(v));
    }
    serde_json::Value::Object(map)
}

pub fn struct_to_json_map(s: &PbStruct) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::with_capacity(s.fields.len());
    for (k, v) in &s.fields {
        map.insert(k.clone(), value_to_json(v));
    }
    map
}

fn value_to_json(v: &PbValue) -> serde_json::Value {
    match &v.kind {
        None => serde_json::Value::Null,
        Some(PbKind::NullValue(_)) => serde_json::Value::Null,
        Some(PbKind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(PbKind::NumberValue(n)) => number_value_to_json(*n),
        Some(PbKind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(PbKind::StructValue(st)) => struct_to_json(st),
        Some(PbKind::ListValue(lv)) => {
            serde_json::Value::Array(lv.values.iter().map(value_to_json).collect())
        }
    }
}

/// Protobuf's `Value` carries all numbers as `f64`, but adapters frequently
/// branch on `serde_json::Value::as_u64`/`as_i64` (e.g. valve indices,
/// motor speeds, slave addresses). Without this coercion every `-p key=3`
/// from the CLI arrives at the driver as `3.0` and fails the integer
/// check. We round-trip integers as integers when the f64 represents one
/// exactly, matching what grpc-gateway and similar JSON↔proto bridges do.
fn number_value_to_json(n: f64) -> serde_json::Value {
    if n.is_finite() && n.fract() == 0.0 {
        if n >= 0.0 && n <= u64::MAX as f64 {
            return serde_json::Value::Number((n as u64).into());
        }
        if n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            return serde_json::Value::Number((n as i64).into());
        }
    }
    serde_json::Number::from_f64(n)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

pub fn json_to_struct(v: &serde_json::Value) -> PbStruct {
    match v {
        serde_json::Value::Object(map) => PbStruct {
            fields: map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect(),
        },
        _ => PbStruct {
            fields: std::collections::BTreeMap::from([(
                "_value".to_string(),
                json_to_value(v),
            )]),
        },
    }
}

pub fn hashmap_to_struct(m: &HashMap<String, serde_json::Value>) -> PbStruct {
    PbStruct {
        fields: m
            .iter()
            .map(|(k, v)| (k.clone(), json_to_value(v)))
            .collect(),
    }
}

fn json_to_value(v: &serde_json::Value) -> PbValue {
    let kind = match v {
        serde_json::Value::Null => PbKind::NullValue(NullValue::NullValue as i32),
        serde_json::Value::Bool(b) => PbKind::BoolValue(*b),
        serde_json::Value::Number(n) => {
            PbKind::NumberValue(n.as_f64().unwrap_or(0.0))
        }
        serde_json::Value::String(s) => PbKind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => PbKind::ListValue(ListValue {
            values: arr.iter().map(json_to_value).collect(),
        }),
        serde_json::Value::Object(map) => PbKind::StructValue(PbStruct {
            fields: map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect(),
        }),
    };
    PbValue { kind: Some(kind) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrips_through_struct() {
        // Whole-number floats are coerced back to ints (see
        // `number_value_to_json`); without this, `-p position=3` from
        // the CLI would arrive at the runze driver as `3.0` and fail
        // its `as_u64()` check.
        let original = serde_json::json!({
            "position": 12.5,
            "status": "Idle",
            "errors": [],
            "nested": {"a": 1, "b": null, "c": true, "negative": -7},
        });
        let pb = json_to_struct(&original);
        let back = struct_to_json(&pb);
        assert_eq!(original, back);
    }

    #[test]
    fn whole_number_floats_become_ints_after_roundtrip() {
        // Specifically the case that broke set_valve_position on real
        // hardware: `position=3` shouldn't degrade to `3.0` on the wire.
        let original = serde_json::json!({"position": 3});
        let pb = json_to_struct(&original);
        let back = struct_to_json(&pb);
        let pos = back.get("position").unwrap();
        assert!(pos.is_u64(), "expected integer, got {pos:?}");
        assert_eq!(pos.as_u64(), Some(3));
    }

    #[test]
    fn fractional_floats_stay_floats() {
        let original = serde_json::json!({"volume": 1.5});
        let back = struct_to_json(&json_to_struct(&original));
        let v = back.get("volume").unwrap();
        assert_eq!(v.as_f64(), Some(1.5));
    }
}
