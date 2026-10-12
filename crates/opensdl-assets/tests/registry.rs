mod support;

#[tokio::test]
async fn a_stalled_registry_request_does_not_hold_the_artifact_lock() {
    let temp = tempfile::tempdir().unwrap();
    let cache = opensdl_assets::AssetCache::open(temp.path()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let reference = opensdl_assets::AssetReference::parse(&format!(
        "{}/lab/slow:dev",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let (accepted, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (_connection, _) = listener.accept().await.unwrap();
        let _ = accepted.send(());
        std::future::pending::<()>().await;
    });
    let other = cache.clone();
    let pull = tokio::spawn(async move {
        opensdl_assets::RegistryClient::for_reference(&reference)
            .pull(
                &reference,
                &other,
                &opensdl_assets::RegistryCredentials::Anonymous,
                false,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), received)
        .await
        .unwrap()
        .unwrap();
    let lease = tokio::time::timeout(std::time::Duration::from_secs(2), cache.lock_async()).await;
    pull.abort();
    let _ = pull.await;
    server.abort();
    let _ = server.await;
    assert!(lease.is_ok(), "network request held the artifact lock");
    drop(lease.unwrap().unwrap());
    assert!(cache.list().unwrap().is_empty());
}

use std::fs;
use std::io::Write;

use opensdl_assets::{
    materialize, pack_directory, AssetCache, AssetReference, OciDescriptor, RegistryClient,
    RegistryCredentials, MANIFEST_MEDIA_TYPE, SOURCE_MEDIA_TYPE,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use support::AssetFixture;

#[test]
fn reference_requires_an_explicit_registry_and_tag_or_digest() {
    assert!(AssetReference::parse("localhost:5000/scienceol/robots/ur5e").is_err());
    assert!(AssetReference::parse("scienceol/robots/ur5e:1.0.0").is_err());

    let tagged = AssetReference::parse("localhost:5000/scienceol/robots/ur5e:1.0.0")
        .expect("tagged reference");
    assert_eq!(tagged.registry(), "localhost:5000");
    assert_eq!(tagged.repository(), "scienceol/robots/ur5e");
    assert_eq!(tagged.tag(), Some("1.0.0"));

    let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let pinned = AssetReference::parse(&format!(
        "oci://hub.example.com/scienceol/robots/ur5e@{digest}"
    ))
    .expect("digest reference");
    assert_eq!(pinned.digest(), Some(digest));
    assert_eq!(
        pinned.to_string(),
        format!("oci://hub.example.com/scienceol/robots/ur5e@{digest}")
    );
}

#[tokio::test]
#[ignore = "requires an explicitly configured disposable OCI registry"]
async fn stock_oci_registry_push_pull_round_trip() {
    let registry = std::env::var("OPENSDL_TEST_REGISTRY")
        .expect("set OPENSDL_TEST_REGISTRY, for example localhost:5001");
    let reference =
        AssetReference::parse(&format!("{registry}/opensdl/test-assets/ur5e:integration"))
            .expect("integration reference");
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create integration root");
    let layout = temp.path().join("layout");
    let packed = pack_directory(&fixture.root, &layout).expect("pack fixture");
    let credentials = RegistryCredentials::Anonymous;
    let client = RegistryClient::for_reference(&reference);

    let pushed = client
        .push_layout(&layout, &reference, &credentials)
        .await
        .expect("push asset");
    assert_eq!(pushed.manifest_digest, packed.manifest_digest);

    let cache = AssetCache::open(temp.path().join("pulled-cache")).expect("open pull cache");
    let pulled = client
        .pull(&reference, &cache, &credentials, false)
        .await
        .expect("pull asset");
    assert_eq!(pulled.resolved_digest, packed.manifest_digest);
    assert!(pulled.downloaded_blobs >= 1);
    assert_eq!(
        materialize(&cache, &pulled.resolved_digest, &temp.path().join("pulled"))
            .expect("materialize pulled asset")
            .manifest
            .metadata
            .name,
        "ur5e"
    );
}

#[test]
fn corruption_verification_is_read_only_and_repair_quarantines_old_bytes() {
    let temp = tempfile::tempdir().expect("create cache root");
    let cache = AssetCache::open(temp.path()).expect("open cache");
    let bytes = b"verified bytes";
    let descriptor = descriptor(SOURCE_MEDIA_TYPE, bytes);
    cache.store_bytes(&descriptor, bytes).expect("store blob");
    assert!(cache.contains(&descriptor).expect("valid cache hit"));

    fs::write(
        cache.blob_path(&descriptor.digest).expect("blob path"),
        b"corrupt",
    )
    .expect("corrupt cache");

    assert!(!cache.contains(&descriptor).expect("corrupt cache miss"));
    assert_eq!(
        fs::read(cache.blob_path(&descriptor.digest).unwrap()).unwrap(),
        b"corrupt"
    );
    assert_eq!(fs::read_dir(cache.quarantine_dir()).unwrap().count(), 0);
    cache.store_bytes(&descriptor, bytes).expect("repair blob");
    assert!(cache.contains(&descriptor).unwrap());
    assert_eq!(
        fs::read_dir(cache.quarantine_dir())
            .expect("read quarantine")
            .count(),
        1
    );
}

#[test]
fn cached_package_materializes_to_a_valid_portable_asset() {
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create materialization root");
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).expect("pack fixture");
    let cache = AssetCache::open(temp.path().join("cache")).expect("open cache");
    cache.import_layout(&layout).expect("import layout");
    let destination = temp.path().join("materialized");

    let asset = materialize(&cache, &package.manifest_digest, &destination)
        .expect("materialize cached package");

    assert_eq!(asset.manifest.metadata.name, "ur5e");
    assert_eq!(
        fs::read(destination.join("scene.usda")).expect("source"),
        b"#usda 1.0\n"
    );
    assert!(destination.join("preview.glb").is_file());
    assert!(destination.join("thumbnail.png").is_file());
}

#[test]
fn materialization_rejects_archive_path_escape() {
    let fixture = AssetFixture::visual();
    let temp = tempfile::tempdir().expect("create materialization root");
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).expect("pack fixture");
    let cache = AssetCache::open(temp.path().join("cache")).expect("open cache");
    cache.import_layout(&layout).expect("import layout");

    let malicious_source = malicious_source_layer();
    let malicious_source_descriptor = descriptor(SOURCE_MEDIA_TYPE, &malicious_source);
    cache
        .store_bytes(&malicious_source_descriptor, &malicious_source)
        .expect("store malicious source");

    let manifest_path = layout
        .join("blobs/sha256")
        .join(package.manifest_digest.trim_start_matches("sha256:"));
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(manifest_path).expect("read manifest"))
            .expect("manifest JSON");
    manifest["layers"][0]["digest"] = malicious_source_descriptor.digest.clone().into();
    manifest["layers"][0]["size"] = malicious_source_descriptor.size.into();
    let malicious_manifest = serde_json::to_vec(&manifest).expect("serialize malicious manifest");
    let malicious_manifest_descriptor = descriptor(MANIFEST_MEDIA_TYPE, &malicious_manifest);
    cache
        .store_bytes(&malicious_manifest_descriptor, &malicious_manifest)
        .expect("store malicious manifest");

    let destination = temp.path().join("materialized");
    let error = materialize(&cache, &malicious_manifest_descriptor.digest, &destination)
        .expect_err("archive escape fails");

    assert!(error.to_string().contains("archive path escapes"));
    assert!(!temp.path().join("escape.txt").exists());
    assert!(!destination.exists());
}

fn descriptor(media_type: &str, bytes: &[u8]) -> OciDescriptor {
    OciDescriptor {
        media_type: media_type.to_owned(),
        digest: format!("sha256:{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
        artifact_type: None,
        annotations: Default::default(),
    }
}

fn malicious_source_layer() -> Vec<u8> {
    let contents = b"escaped";
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    let path = b"../escape.txt";
    header.as_mut_bytes()[..path.len()].copy_from_slice(path);
    header.set_cksum();

    let mut tar_bytes = Vec::new();
    tar_bytes.extend_from_slice(header.as_bytes());
    tar_bytes.extend_from_slice(contents);
    tar_bytes.resize(1024, 0);
    tar_bytes.extend_from_slice(&[0_u8; 1024]);

    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 10).expect("zstd encoder");
    encoder.write_all(&tar_bytes).expect("compress tar");
    encoder.finish().expect("finish zstd")
}
