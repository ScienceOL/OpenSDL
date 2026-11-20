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
                package.asset.metadata.name,
                package.manifest_digest
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
