use osdl_core::media::onvif_camera::SNAPSHOT_SUBDIR;
use osdl_proto::v1 as pb;
use prost_types::{value::Kind, Struct};
use tonic::Status;

use crate::config::Id;

pub(super) fn split(routed_id: &str) -> Result<(Id, &str), Status> {
    let (worker, id) = routed_id
        .split_once(':')
        .filter(|(_, id)| !id.is_empty())
        .ok_or_else(|| Status::invalid_argument("expected worker:id"))?;
    let worker = worker
        .parse()
        .map_err(|_| Status::invalid_argument("invalid worker ID"))?;
    Ok((worker, id))
}

pub(super) fn qualify(worker: &Id, id: &mut String) {
    if !id.is_empty() {
        *id = format!("{worker}:{id}");
    }
}

pub(super) fn device(worker: &Id, device: &mut pb::Device) {
    properties(worker, &device.id, &mut device.properties);
    qualify(worker, &mut device.id);
    // This is a worker-local address, not an identifier used by the gateway.
    device.transport_id.clear();
}

pub(super) fn node(worker: &Id, node: &mut pb::Node) {
    qualify(worker, &mut node.node_id);
    if let Some(id) = &mut node.device_id {
        qualify(worker, id);
    }
}

pub(super) fn command_result(worker: &Id, result: &mut pb::CommandResult) {
    properties(worker, &result.device_id, &mut result.data);
    if let Some(fields) = result.data.as_mut().map(|data| &mut data.fields) {
        if let Some(Kind::StringValue(id)) = fields
            .get_mut("device_id")
            .and_then(|value| value.kind.as_mut())
        {
            if *id == result.device_id {
                qualify(worker, id);
            }
        }
    }
    qualify(worker, &mut result.device_id);
    // Caller-supplied command_id remains the correlation key.
}

pub(super) fn properties(worker: &Id, id: &str, properties: &mut Option<Struct>) {
    let Some(properties) = properties.as_mut() else {
        return;
    };
    for key in [
        "snapshot_url",
        "snapshot_source_uri",
        "snapshot_error",
        "ptz_move_error",
        "ptz_stop_error",
        "ptz_preset_goto_error",
    ] {
        properties.fields.remove(key);
    }
    if let Some(mut path) = properties.fields.remove("snapshot_path") {
        if let Some(Kind::StringValue(value)) = path.kind.as_mut() {
            if let Some(relative) = snapshot_relative(id, value) {
                *value = format!("{worker}:{relative}");
                properties.fields.insert("snapshot_path".into(), path);
            }
        }
    }
}

fn snapshot_relative(camera: &str, path: &str) -> Option<String> {
    // The worker snapshot layout is snapshots/<camera>/<epoch-ms>.jpg.
    // Retain only the worker-relative locator even when the worker reports
    // an absolute Unix or Windows path.
    let mut parts = path.rsplit(['/', '\\']);
    let file = parts.next()?;
    let parent = parts.next()?;
    if parent != camera || parts.next().is_some_and(|root| root != SNAPSHOT_SUBDIR) {
        return None;
    }
    let relative = format!("{parent}/{file}");
    snapshot_path(&relative).ok()?;
    Some(relative)
}

pub(super) fn snapshot_path(path: &str) -> Result<&str, Status> {
    let valid = path.split_once('/').is_some_and(|(camera, file)| {
        !camera.is_empty()
            && camera
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            && file.strip_suffix(".jpg").is_some_and(|name| {
                (1..=20).contains(&name.len()) && name.bytes().all(|c| c.is_ascii_digit())
            })
    });
    if valid && path.len() <= 4096 {
        Ok(path)
    } else {
        Err(Status::invalid_argument("invalid snapshot locator"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osdl_server::convert::{json_to_struct, struct_to_json};
    use serde_json::json;

    #[test]
    fn colliding_ids_are_routed_by_the_first_colon() {
        for name in ["alpha", "beta"] {
            let worker = name.parse().unwrap();
            let mut id = "sim:lab:heater".into();
            qualify(&worker, &mut id);
            assert_eq!(id, format!("{name}:sim:lab:heater"));
            assert_eq!(split(&id).unwrap(), (worker, "sim:lab:heater"));
        }
        for invalid in ["heater", "alpha:", ":heater", "../alpha:heater"] {
            assert!(split(invalid).is_err());
        }
    }

    #[test]
    fn command_results_keep_the_callers_correlation_id() {
        let worker = "alpha".parse().unwrap();
        let mut result = pb::CommandResult {
            command_id: "client:123".into(),
            device_id: "sim:lab:heater".into(),
            data: Some(json_to_struct(
                &json!({"device_id": "sim:lab:heater", "timestamp": 123}),
            )),
            ..Default::default()
        };
        command_result(&worker, &mut result);
        assert_eq!(result.device_id, "alpha:sim:lab:heater");
        assert_eq!(result.command_id, "client:123");
        assert_eq!(
            struct_to_json(result.data.as_ref().unwrap())["device_id"],
            result.device_id
        );
    }

    #[test]
    fn snapshot_only_exposes_a_valid_worker_relative_locator() {
        for original in [
            "/home/service/snapshots/cam131/1234.jpg",
            r"C:\service\snapshots\cam131\1234.jpg",
            "cam131/1234.jpg",
        ] {
            let mut device = pb::Device {
                id: "cam131".into(),
                transport_id: "192.0.2.1:1234".into(),
                properties: Some(json_to_struct(&json!({
                    "snapshot_path": original,
                    "snapshot_source_uri": "http://camera.example/private",
                    "snapshot_url": "file:///private",
                    "snapshot_ok": true,
                    "ptz_move_error": "http://camera.example/control",
                }))),
                ..Default::default()
            };
            super::device(&"scienceol".parse().unwrap(), &mut device);
            assert!(device.transport_id.is_empty());
            assert_eq!(
                struct_to_json(device.properties.as_ref().unwrap()),
                json!({"snapshot_path": "scienceol:cam131/1234.jpg", "snapshot_ok": true})
            );
        }
    }

    #[test]
    fn invalid_snapshot_routes_and_noncanonical_upstream_paths_are_rejected() {
        for invalid in [
            "/cam131/1234.jpg",
            "../cam131/1234.jpg",
            "cam131/../1234.jpg",
            r"cam131\1234.jpg",
            "cam131/1234.png",
            "cam131/1.jpg:stream",
        ] {
            assert!(snapshot_path(invalid).is_err(), "{invalid}");
        }
        assert!(snapshot_relative("cam131", "/foo/snapshots/cam53/1234.jpg").is_none());
        assert!(snapshot_relative("cam131", "/foo/other/cam131/1234.jpg").is_none());
    }
}
