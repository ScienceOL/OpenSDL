mod support;
use serde_json::{json, Value};
use support::instruments::{last, send, setup, v};

#[tokio::test]
async fn all_authored_runtime_observations_match_their_exported_joint_motion_contract() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/instruments");
    let mut instruments = 0;
    let mut rotating = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let slug = entry.file_name().into_string().unwrap();
        let (world, mut rx, id) = setup(&slug);
        let snapshot = send(&world, &id, "reset", json!({})).await;
        assert_eq!(v(&last(&mut rx)), v(&snapshot), "{slug} emitted state");
        let state = &snapshot["properties"]["model_state"];
        assert_eq!(state["schema_version"], 2, "{slug}");
        assert_eq!(state.as_object().unwrap().len(), 7);
        let motion = state["motion"].as_object().unwrap();
        let bytes = std::fs::read(entry.path().join("package/preview.glb")).unwrap();
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let gltf: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let graph = gltf["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|n| n.get("extras")?.get("opensdlModel"))
            .unwrap();
        assert_eq!(state["model_id"], graph["model_id"], "{slug}");
        let mut velocity_keys = std::collections::HashSet::new();
        for joint in graph["joints"].as_array().unwrap() {
            if joint["kind"] != "continuous" {
                continue;
            }
            let key = joint["velocity_state"].as_str().unwrap();
            velocity_keys.insert(key);
            let profile = &motion[key];
            assert_eq!(profile.as_object().unwrap().len(), 3);
            assert!(
                v(&snapshot)[key].as_f64().unwrap().is_finite(),
                "{slug} {key}"
            );
            assert!(profile["target_velocity"].as_f64().unwrap().is_finite());
            assert!(profile["slew_per_s"].as_f64().unwrap() > 0.0);
            assert!(
                profile["stop_after_s"].is_null()
                    || profile["stop_after_s"].as_f64().unwrap() >= 0.0
            );
        }
        assert_eq!(
            motion.len(),
            velocity_keys.len(),
            "unbound trajectory in {slug}"
        );
        rotating += usize::from(!velocity_keys.is_empty());
        instruments += 1;
    }
    assert_eq!((instruments, rotating), (22, 8));
}
