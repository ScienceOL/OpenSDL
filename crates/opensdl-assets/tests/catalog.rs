mod support;

use opensdl_assets::{pack_directory, AssetCache, AssetReference};
use support::AssetFixture;

#[test]
fn pruning_keeps_shared_blobs_owned_by_a_tagged_package() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = AssetFixture::visual();
    let first = pack_directory(&fixture.root, &temp.path().join("one")).unwrap();
    let yaml = std::fs::read_to_string(fixture.root.join("asset.yaml"))
        .unwrap()
        .replace("version: 1.0.0", "version: 2.0.0");
    fixture.replace_manifest(&yaml);
    let second = pack_directory(&fixture.root, &temp.path().join("two")).unwrap();
    let cache = AssetCache::open(temp.path().join("store")).unwrap();
    cache.import_layout(&temp.path().join("one")).unwrap();
    cache.import_layout(&temp.path().join("two")).unwrap();
    let reference = AssetReference::parse("localhost/lab/ur5e:2.0.0").unwrap();
    cache.tag(&second.manifest_digest, &reference).unwrap();
    let report = cache.prune(true).unwrap();
    assert_eq!(report.packages, vec![first.manifest_digest.clone()]);
    assert!(cache.package(&first.manifest_digest).is_ok());
    cache.prune(false).unwrap();
    assert!(cache.package(&first.manifest_digest).is_err());
    assert!(cache.package(&second.manifest_digest).is_ok());
    assert_eq!(
        cache.resolve_reference(&reference).unwrap(),
        second.manifest_digest
    );
}

#[test]
fn a_retained_owner_protects_an_untagged_package() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = AssetFixture::visual();
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).unwrap();
    let cache = AssetCache::open(temp.path().join("store")).unwrap();
    cache.import_layout(&layout).unwrap();
    cache
        .pin("instance:test", &package.manifest_digest)
        .unwrap();
    assert!(cache.remove(&package.manifest_digest).is_err());
    assert!(cache.prune(false).unwrap().packages.is_empty());
    assert!(cache.package(&package.manifest_digest).is_ok());
    cache.unpin("instance:test").unwrap();
    assert_eq!(
        cache.prune(false).unwrap().packages,
        vec![package.manifest_digest]
    );
}

#[test]
fn pruning_refuses_a_dangling_owner_instead_of_deleting_its_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = AssetFixture::visual();
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).unwrap();
    let cache = AssetCache::open(temp.path().join("store")).unwrap();
    cache.import_layout(&layout).unwrap();
    cache
        .pin("instance:test", &package.manifest_digest)
        .unwrap();
    let connection = rusqlite::Connection::open(cache.catalog_path()).unwrap();
    connection
        .execute(
            "DELETE FROM package WHERE digest=?1",
            [&package.manifest_digest],
        )
        .unwrap();
    assert!(cache.prune(false).is_err());
    assert!(cache.package(&package.manifest_digest).is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_for_the_artifact_lock_does_not_block_the_async_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let cache = AssetCache::open(temp.path()).unwrap();
    let held = cache.lock().unwrap();
    let other = cache.clone();
    let pending = tokio::spawn(async move { other.lock_async().await });
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(!pending.is_finished());
    drop(held);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}

#[test]
fn gc_uses_the_immutable_manifest_when_catalog_layer_metadata_is_damaged() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = AssetFixture::visual();
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).unwrap();
    let cache = AssetCache::open(temp.path().join("store")).unwrap();
    cache.import_layout(&layout).unwrap();
    cache
        .pin("instance:test", &package.manifest_digest)
        .unwrap();
    let mut corrupted = serde_json::to_value(&package).unwrap();
    corrupted["layers"][0]["digest"] = serde_json::json!(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    let connection = rusqlite::Connection::open(cache.catalog_path()).unwrap();
    connection
        .execute(
            "UPDATE package SET descriptor=?1 WHERE digest=?2",
            rusqlite::params![corrupted.to_string(), package.manifest_digest],
        )
        .unwrap();
    let stray = cache
        .blob_path(&format!("sha256:{}", "b".repeat(64)))
        .unwrap();
    std::fs::write(&stray, b"unreferenced").unwrap();
    cache.prune(false).unwrap();
    assert!(!stray.exists());
    assert!(cache.package(&package.manifest_digest).is_ok());
}

#[test]
fn removed_inventory_entries_cannot_be_resolved_or_pinned_from_orphaned_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = AssetFixture::visual();
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).unwrap();
    let cache = AssetCache::open(temp.path().join("store")).unwrap();
    cache.import_layout(&layout).unwrap();
    cache.remove(&package.manifest_digest).unwrap();
    assert!(cache.package(&package.manifest_digest).is_ok());
    assert!(matches!(
        cache.resolve(&package.manifest_digest),
        Err(opensdl_assets::AssetError::NotFound(_))
    ));
    assert!(cache.pin("instance:new", &package.manifest_digest).is_err());
    assert!(cache.list().unwrap().is_empty());
    cache.prune(false).unwrap();
}

#[test]
fn dry_run_preserves_corrupt_retained_blobs() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = AssetFixture::visual();
    let layout = temp.path().join("layout");
    let package = pack_directory(&fixture.root, &layout).unwrap();
    let cache = AssetCache::open(temp.path().join("store")).unwrap();
    cache.import_layout(&layout).unwrap();
    cache
        .pin("instance:retained", &package.manifest_digest)
        .unwrap();
    let blob = cache.blob_path(&package.config.digest).unwrap();
    std::fs::write(&blob, b"damaged").unwrap();
    assert!(cache.prune(true).is_err());
    assert_eq!(std::fs::read(blob).unwrap(), b"damaged");
    assert_eq!(
        std::fs::read_dir(cache.quarantine_dir()).unwrap().count(),
        0
    );
}

#[test]
fn opening_a_store_refuses_an_unrelated_sqlite_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let database = rusqlite::Connection::open(&path).unwrap();
    database
        .execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES('keep');")
        .unwrap();
    assert!(AssetCache::open(temp.path()).is_err());
    assert_eq!(
        database
            .query_row("SELECT value FROM unrelated", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn store_owned_directories_cannot_redirect_to_unrelated_files() {
    let temp = tempfile::tempdir().unwrap();
    let store = temp.path().join("store");
    let unrelated = temp.path().join("unrelated");
    std::fs::create_dir(&store).unwrap();
    std::fs::create_dir(&unrelated).unwrap();
    std::os::unix::fs::symlink(&unrelated, store.join("blobs")).unwrap();
    assert!(AssetCache::open(&store).is_err());
    assert_eq!(std::fs::read_dir(&unrelated).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn the_catalog_is_private_even_inside_an_existing_public_directory() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let cache = AssetCache::open(temp.path()).unwrap();
    assert_eq!(
        std::fs::metadata(cache.catalog_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
