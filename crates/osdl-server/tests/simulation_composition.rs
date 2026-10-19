use opensdl_assets::{pack_directory, AssetCache};

#[test]
fn hub_instruments_form_one_valid_world_with_distinct_devices_and_pinned_models() {
    let temp = tempfile::tempdir().unwrap();
    let cache = AssetCache::open(temp.path().join("cache")).unwrap();
    let sources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/instruments");
    let mut references = Vec::new();
    for entry in std::fs::read_dir(sources).unwrap() {
        let source = entry.unwrap().path().join("package");
        if !source.join("asset.yaml").exists() {
            continue;
        }
        let layout = temp.path().join(format!("layout-{}", references.len()));
        let package = pack_directory(&source, &layout).unwrap();
        cache.import_layout(&layout).unwrap();
        references.push(format!(
            "hub.liyanlabs.com/opensdl/test@{}",
            package.manifest_digest
        ));
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
    let duplicated = osdl_server::simulation_asset::compose(
        &cache,
        &vec![references[0].clone(); 2],
        &temp.path().join("duplicate"),
        "two",
    )
    .unwrap();
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
        &["hub.liyanlabs.com/lab/test:latest".into()],
        &temp.path().join("mutable"),
        "mutable"
    )
    .is_err());
}
