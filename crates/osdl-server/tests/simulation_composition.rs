use opensdl_assets::{pack_directory, AssetCache};

#[test]
fn hub_instruments_form_one_valid_world_with_distinct_devices_and_pinned_models() {
    let temp = tempfile::tempdir().unwrap();
    let cache = AssetCache::open(temp.path().join("cache")).unwrap();
    let sources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/instruments");
    let mut references = Vec::new();
    use osdl_server::simulation_asset::SceneInstrument;
    for entry in std::fs::read_dir(sources).unwrap() {
        let source = entry.unwrap().path().join("package");
        if !source.join("asset.yaml").exists() {
            continue;
        }
        let layout = temp.path().join(format!("layout-{}", references.len()));
        let package = pack_directory(&source, &layout).unwrap();
        cache.import_layout(&layout).unwrap();
        references.push(SceneInstrument {
            id: format!("device-{}", references.len()),
            version: package.asset.metadata.version.clone(),
            position: [references.len() as f64, 0., 0.],
            rotation: [0., 1., 0.],
            parent: None,
            asset: format!(
                "hub.liyanlabs.com/opensdl/{}@{}",
                package.asset.metadata.name, package.manifest_digest
            ),
        });
    }
    assert_eq!(references.len(), 22);
    let world = osdl_server::simulation_asset::compose(
        &cache,
        &references,
        &temp.path().join("world"),
        "my-lab",
    )
    .unwrap();
    world.validate().unwrap();
    assert_eq!(world.world_id, "my-lab");
    assert_eq!(world.devices.len(), references.len());
    for device in &world.devices {
        assert!(device.asset_ref.is_some());
        assert!(device.properties["model_manifest_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(device.model.is_some());
    }
    // The same immutable instrument may be instantiated twice without id collisions.
    let mut second = references[0].clone();
    second.id = "another".into();
    second.position = [0., 1., 2.];
    second.parent = Some(references[0].id.clone());
    let duplicated = osdl_server::simulation_asset::compose(
        &cache,
        &[references[0].clone(), second],
        &temp.path().join("duplicate"),
        "two",
    )
    .unwrap();
    assert_eq!(duplicated.devices[1].position, [0., 1., 2.]);
    assert_eq!(
        duplicated.devices[1].properties["rotation"],
        serde_json::json!([0., 1., 0.])
    );
    assert_eq!(
        duplicated.devices[1].properties["parent"],
        serde_json::json!(format!("sim:two:{}", references[0].id))
    );
    assert_ne!(duplicated.devices[0].id, duplicated.devices[1].id);
    assert_ne!(
        duplicated.devices[0].position,
        duplicated.devices[1].position
    );
    assert!(osdl_server::simulation_asset::compose(
        &cache,
        &[],
        &temp.path().join("empty"),
        "empty"
    )
    .is_err());
    assert!(osdl_server::simulation_asset::compose(
        &cache,
        &[SceneInstrument {
            asset: "hub.liyanlabs.com/lab/test:latest".into(),
            ..references[0].clone()
        }],
        &temp.path().join("mutable"),
        "mutable"
    )
    .is_err());
}

#[tokio::test]
async fn rejected_composition_preserves_live_world_and_prepared_edits_apply_once() {
    use osdl_core::config::OsdlConfig;
    use osdl_proto::v1::{osdl_server::Osdl, ApplySceneRequest, PrepareSceneRequest};
    use osdl_server::{
        simulation_asset::{compose, SceneDefinition},
        OsdlService, ServerIdentity,
    };
    use serde_json::json;
    use std::fs;
    use tonic::{Code, Request};

    let data = tempfile::tempdir().unwrap();
    let cache = AssetCache::open(data.path().join("assets")).unwrap();
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/instruments/analytical-balance/package");
    let import = |source: &std::path::Path, layout: &str| {
        let layout = data.path().join(layout);
        let digest = pack_directory(source, &layout).unwrap().manifest_digest;
        cache.import_layout(&layout).unwrap();
        digest
    };
    let digest = import(&package, "layout-1");
    let variant = data.path().join("variant");
    cache
        .lock()
        .unwrap()
        .materialize_package(&digest, &variant)
        .unwrap();
    let config = variant.join("source/world.yaml");
    let mut definition: serde_json::Value =
        serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    definition["samples"][0]["mass_kg"] = json!(0.05);
    fs::write(config, serde_json::to_vec(&definition).unwrap()).unwrap();
    let other = import(&variant, "layout-2");
    let instrument = |id: &str, digest: &str| {
        json!({
            "id":id, "asset":format!("hub.liyanlabs.com/opensdl/analytical-balance@{digest}"),
            "version":"1.1.2", "position":[0,0,0], "rotation":[0,0,0]
        })
    };
    let first = instrument("balance", &digest);
    let stage = |items: serde_json::Value| {
        let path = data.path().join("scene-2.json");
        fs::write(
            &path,
            json!({"name":"Bench","instruments":items}).to_string(),
        )
        .unwrap();
        SceneDefinition::read(&path).unwrap()
    };
    let layouts = data.path().join("world");
    let simulation = compose(&cache, &stage(json!([first])).instruments, &layouts, "lab").unwrap();
    let mut engine = osdl_core::OsdlEngine::new(
        OsdlConfig {
            simulation: Some(simulation),
            mqtt: None,
            data_dir: Some(data.path().to_owned()),
            ..Default::default()
        },
        vec![],
    );
    let handle = engine.handle();
    let task = tokio::spawn(async move { engine.run().await });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while handle.get_device("sim:lab:balance").await.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let service = OsdlService::new(
        handle.clone(),
        ServerIdentity {
            instance: "lab".into(),
            version: "test".into(),
            pid: 1,
            started_at: Default::default(),
            socket_path: None,
            listen_addr: None,
        },
    );
    let prepare = || service.prepare_scene(Request::new(PrepareSceneRequest { revision: 2 }));
    let apply = |ticket: String| service.apply_scene(Request::new(ApplySceneRequest { ticket }));
    stage(json!([first, instrument("other", &other)]));
    let err = prepare().await.unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(
        err.message().contains("conflicting sample fixture"),
        "{err}"
    );
    assert_eq!(handle.list_devices().await.len(), 1);
    stage(json!([first, instrument("other", &digest)]));
    let first_ticket = prepare().await.unwrap().into_inner().ticket;
    let ticket = prepare().await.unwrap().into_inner().ticket;
    assert_eq!(apply(first_ticket).await.unwrap_err().code(), Code::Aborted);
    assert_eq!(handle.list_devices().await.len(), 1);
    apply(ticket.clone()).await.unwrap();
    assert_eq!(handle.list_devices().await.len(), 2);
    assert_eq!(apply(ticket).await.unwrap_err().code(), Code::Aborted);
    handle.request_stop();
    task.await.unwrap();
}
