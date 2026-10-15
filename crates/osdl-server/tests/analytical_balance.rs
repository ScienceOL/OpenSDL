use opensdl_assets::{pack_directory, AssetCache};
#[test]
fn original_package_preserves_geometry_parameters_and_digest_binding() {
    let temp = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/instruments/analytical-balance/package");
    let layout = temp.path().join("layout");
    let package = pack_directory(&source, &layout).unwrap();
    let cache = AssetCache::open(temp.path().join("cache")).unwrap();
    cache.import_layout(&layout).unwrap();
    let destination = temp.path().join("prepared");
    let config =
        osdl_server::simulation_asset::prepare(&cache, &package.manifest_digest, &destination)
            .unwrap();
    assert_eq!(
        config.devices[0].properties["model_manifest_digest"],
        package.manifest_digest
    );
    assert_eq!(
        std::fs::read(source.join("preview.glb")).unwrap(),
        std::fs::read(destination.join("preview.glb")).unwrap()
    );
}
