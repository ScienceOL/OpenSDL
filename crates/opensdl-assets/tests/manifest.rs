mod support;

use std::fs;

use opensdl_assets::{AssetType, Capability, SourceFormat, VirtualAssetManifest};
use support::{base_manifest, write_minimal_glb, AssetFixture};

fn visual_gltf_manifest(source_root: &str) -> String {
    let manifest = base_manifest()
        .replace("format: openusd", "format: gltf")
        .replace("root: scene.usda", &format!("root: {source_root}"))
        .replace("    - simulation\n", "")
        .replace("    - opensdl-device\n", "");
    let binding_start = manifest.find("  bindings:\n").expect("bindings section");
    manifest[..binding_start].to_owned()
}

#[test]
fn valid_openusd_asset_exposes_only_validated_paths() {
    let fixture = AssetFixture::visual();

    let asset = VirtualAssetManifest::load(&fixture.root).expect("valid asset");

    assert_eq!(asset.manifest.metadata.name, "ur5e");
    assert_eq!(asset.manifest.spec.asset_type, AssetType::Robot);
    assert_eq!(asset.manifest.spec.source.format, SourceFormat::OpenUsd);
    assert!(asset
        .manifest
        .spec
        .capabilities
        .contains(&Capability::Simulation));
    assert_eq!(asset.paths.source_root, asset.root.join("scene.usda"));
    assert_eq!(asset.paths.preview_model, asset.root.join("preview.glb"));
}

#[test]
fn rejects_parent_escape_and_unknown_fields() {
    let parent_escape = AssetFixture::visual();
    parent_escape
        .replace_manifest(&base_manifest().replace("root: scene.usda", "root: ../scene.usda"));
    let error = VirtualAssetManifest::load(&parent_escape.root).expect_err("parent escape fails");
    assert!(error.to_string().contains("path escapes asset root"));

    let unknown_field = AssetFixture::visual();
    unknown_field.replace_manifest(
        &base_manifest().replace("  type: robot", "  type: robot\n  mystery: value"),
    );
    let error = VirtualAssetManifest::load(&unknown_field.root).expect_err("unknown field fails");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn gltf_cannot_claim_simulation() {
    let fixture = AssetFixture::visual();
    fs::remove_file(fixture.root.join("scene.usda")).expect("remove OpenUSD source");
    write_minimal_glb(&fixture.root.join("source.glb"), b"{ }\0");
    fixture.replace_manifest(
        &base_manifest()
            .replace("format: openusd", "format: gltf")
            .replace("root: scene.usda", "root: source.glb"),
    );

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("gltf simulation fails");
    assert!(error.to_string().contains("gltf cannot claim simulation"));
}

#[test]
fn rejects_non_dns_label_name() {
    let fixture = AssetFixture::visual();
    fixture.replace_manifest(&base_manifest().replace("name: ur5e", "name: UR5E"));

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("invalid identity fails");
    assert!(error.to_string().contains("DNS label"));
}

#[test]
fn rejects_dns_label_with_non_alphanumeric_characters() {
    let fixture = AssetFixture::visual();
    fixture.replace_manifest(&base_manifest().replace("name: ur5e", "name: ur5e_bad"));

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("invalid DNS character fails");
    assert!(error.to_string().contains("DNS label"));
}

#[test]
fn rejects_non_semver_version() {
    let fixture = AssetFixture::visual();
    fixture.replace_manifest(&base_manifest().replace("version: 1.0.0", "version: v1.0"));

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("invalid version fails");
    assert!(error.to_string().contains("semantic version"));
}

#[test]
fn rejects_invalid_spdx_expression() {
    let fixture = AssetFixture::visual();
    fixture
        .replace_manifest(&base_manifest().replace("spdx: CC-BY-4.0", "spdx: definitely-not-spdx"));

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("invalid SPDX fails");
    assert!(error.to_string().contains("SPDX expression"));
}

#[test]
fn opensdl_device_capability_requires_a_binding() {
    let fixture = AssetFixture::visual();
    let manifest = base_manifest();
    let binding_start = manifest.find("  bindings:\n").expect("bindings section");
    fixture.replace_manifest(&manifest[..binding_start]);

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("missing binding fails");
    assert!(error.to_string().contains("opensdl-device requires"));
}

#[test]
fn rejects_uri_and_absolute_manifest_paths() {
    for source_root in ["https://example.com/scene.usda", "/tmp/scene.usda"] {
        let fixture = AssetFixture::visual();
        fixture.replace_manifest(
            &base_manifest().replace("root: scene.usda", &format!("root: {source_root}")),
        );

        let error = VirtualAssetManifest::load(&fixture.root).expect_err("non-portable path fails");
        assert!(error.to_string().contains("relative POSIX path"));
    }
}

#[test]
fn rejects_corrupt_preview_glb() {
    let fixture = AssetFixture::visual();
    fs::write(fixture.root.join("preview.glb"), b"not a GLB").expect("corrupt preview");

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("corrupt GLB fails");
    assert!(error.to_string().contains("preview.glb"));
}

#[test]
fn rejects_preview_with_the_same_bytes_as_the_glb_source() {
    let fixture = AssetFixture::visual();
    fs::copy(
        fixture.root.join("preview.glb"),
        fixture.root.join("source.glb"),
    )
    .expect("copy preview as source");
    fixture.replace_manifest(&visual_gltf_manifest("source.glb"));

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("identical preview fails");
    assert!(error.to_string().contains("same bytes"));
}

#[test]
fn rejects_gltf_dependency_outside_the_source_directory() {
    let fixture = AssetFixture::visual();
    let source_dir = fixture.root.join("models");
    fs::create_dir(&source_dir).expect("create source directory");
    fs::write(
        source_dir.join("source.gltf"),
        br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"../outside.bin","byteLength":4}]}"#,
    )
    .expect("write glTF source");
    fs::write(fixture.root.join("outside.bin"), [0_u8; 4]).expect("write external dependency");
    fixture.replace_manifest(&visual_gltf_manifest("models/source.gltf"));

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("external dependency fails");
    assert!(error.to_string().contains("glTF dependency path escapes"));
}

#[test]
fn rejects_empty_bundled_license_text() {
    let fixture = AssetFixture::visual();
    fs::write(fixture.root.join("LICENSE"), []).expect("empty license");

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("empty license fails");
    assert!(error.to_string().contains("license text must not be empty"));
}

#[cfg(unix)]
#[test]
fn rejects_special_files_inside_the_asset_root() {
    use std::os::unix::net::UnixListener;

    let fixture = AssetFixture::visual();
    let _listener = UnixListener::bind(fixture.root.join("control.sock")).expect("create socket");

    let error = VirtualAssetManifest::load(&fixture.root).expect_err("special file fails");
    assert!(error.to_string().contains("special file"));
}

#[cfg(unix)]
#[test]
fn rejects_links_inside_the_asset_root() {
    use std::os::unix::fs::symlink;

    let symlink_fixture = AssetFixture::visual();
    symlink(
        symlink_fixture.root.join("scene.usda"),
        symlink_fixture.root.join("linked.usda"),
    )
    .expect("create symlink");
    let error = VirtualAssetManifest::load(&symlink_fixture.root).expect_err("symlink fails");
    assert!(error.to_string().contains("symbolic link"));

    let hardlink_fixture = AssetFixture::visual();
    fs::hard_link(
        hardlink_fixture.root.join("scene.usda"),
        hardlink_fixture.root.join("copied.usda"),
    )
    .expect("create hard link");
    let error = VirtualAssetManifest::load(&hardlink_fixture.root).expect_err("hard link fails");
    assert!(error.to_string().contains("hard link"));
}
