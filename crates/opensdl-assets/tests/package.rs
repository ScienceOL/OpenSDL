mod support;

use std::fs;

use opensdl_assets::{
    inspect_layout, pack_directory, ARTIFACT_TYPE, CONFIG_MEDIA_TYPE, MANIFEST_MEDIA_TYPE,
    PREVIEW_MEDIA_TYPE, SOURCE_MEDIA_TYPE,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use support::AssetFixture;

#[test]
fn packing_twice_produces_identical_descriptors() {
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create package output root");

    let first = pack_directory(&fixture.root, &temp.path().join("first")).expect("first pack");
    let second = pack_directory(&fixture.root, &temp.path().join("second")).expect("second pack");

    assert_eq!(first, second);
    assert_eq!(first.artifact_type, ARTIFACT_TYPE);
    assert_eq!(first.manifest_media_type, MANIFEST_MEDIA_TYPE);
    assert_eq!(first.config.media_type, CONFIG_MEDIA_TYPE);
    assert_eq!(
        first
            .layers
            .iter()
            .map(|layer| layer.media_type.as_str())
            .collect::<Vec<_>>(),
        vec![SOURCE_MEDIA_TYPE, PREVIEW_MEDIA_TYPE, "image/png"]
    );
}

#[test]
fn source_bundle_excludes_dedicated_preview_layers() {
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create package output root");
    let output = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &output).expect("pack asset");
    let source = package
        .layers
        .iter()
        .find(|layer| layer.media_type == SOURCE_MEDIA_TYPE)
        .expect("source layer");
    let source_blob = output.join("blobs/sha256").join(
        source
            .digest
            .strip_prefix("sha256:")
            .expect("sha256 digest"),
    );
    let decoder =
        zstd::stream::read::Decoder::new(fs::File::open(source_blob).expect("source blob"))
            .expect("zstd decoder");
    let mut archive = tar::Archive::new(decoder);
    let names = archive
        .entries()
        .expect("tar entries")
        .map(|entry| {
            entry
                .expect("tar entry")
                .path()
                .expect("entry path")
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["LICENSE", "asset.yaml", "scene.usda"]);
    assert!(!names.iter().any(|name| name == "preview.glb"));
    assert!(!names.iter().any(|name| name == "thumbnail.png"));
}

#[test]
fn inspection_verifies_every_blob_digest() {
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create package output root");
    let output = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &output).expect("pack asset");
    assert_eq!(
        inspect_layout(&output).expect("inspect valid layout"),
        package
    );

    let preview = package
        .layers
        .iter()
        .find(|layer| layer.media_type == PREVIEW_MEDIA_TYPE)
        .expect("preview layer");
    let preview_blob = output.join("blobs/sha256").join(
        preview
            .digest
            .strip_prefix("sha256:")
            .expect("sha256 digest"),
    );
    let mut corrupt = fs::read(&preview_blob).expect("read preview blob");
    corrupt[0] ^= 0xff;
    fs::write(preview_blob, corrupt).expect("corrupt preview blob");

    let error = inspect_layout(&output).expect_err("corrupt blob fails inspection");
    assert!(error.to_string().contains("digest mismatch"));
}

#[test]
fn inspection_rejects_semantically_invalid_verified_config() {
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create package output root");
    let output = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &output).expect("pack asset");
    let blob_root = output.join("blobs/sha256");

    let config_path = blob_root.join(hash(&package.config.digest));
    let mut config: Value =
        serde_json::from_slice(&fs::read(config_path).expect("read config")).expect("config JSON");
    config["metadata"]["license"]["spdx"] = "not-an-spdx-expression".into();
    let config_bytes = serde_json::to_vec(&config).expect("serialize invalid config");
    let config_digest = digest(&config_bytes);
    fs::write(blob_root.join(hash(&config_digest)), &config_bytes).expect("write invalid config");

    let manifest_path = blob_root.join(hash(&package.manifest_digest));
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(manifest_path).expect("read manifest"))
            .expect("manifest JSON");
    manifest["config"]["digest"] = config_digest.into();
    manifest["config"]["size"] = config_bytes.len().into();
    let manifest_bytes = serde_json::to_vec(&manifest).expect("serialize manifest");
    let manifest_digest = digest(&manifest_bytes);
    fs::write(blob_root.join(hash(&manifest_digest)), &manifest_bytes).expect("write manifest");

    let index_path = output.join("index.json");
    let mut index: Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read index")).expect("index JSON");
    index["manifests"][0]["digest"] = manifest_digest.into();
    index["manifests"][0]["size"] = manifest_bytes.len().into();
    fs::write(
        index_path,
        serde_json::to_vec(&index).expect("serialize index"),
    )
    .expect("write index");

    let error = inspect_layout(&output).expect_err("invalid SPDX must fail inspection");
    assert!(error.to_string().contains("SPDX expression"));
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn hash(digest: &str) -> &str {
    digest.strip_prefix("sha256:").expect("sha256 digest")
}
