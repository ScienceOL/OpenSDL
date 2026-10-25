use osdl_auth::{Invitation, ProfileStore};
#[test]
fn rejects_untrusted_and_oversized_invitation() {
    assert!(Invitation::parse("not-an-invitation").is_err());
    assert!(Invitation::parse(&"x".repeat(8193)).is_err());
}
#[test]
fn native_store_preserves_pending_key_and_refuses_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let store = ProfileStore::open(temp.path().join("profiles")).unwrap();
    store
        .write_secret("pending", b"private-key-canary", false)
        .unwrap();
    assert_eq!(store.read_secret("pending").unwrap(), b"private-key-canary");
    assert!(store
        .write_secret("pending", b"replacement", false)
        .is_err());
    assert!(store.read_secret("../outside").is_err());
    #[cfg(windows)]
    assert!(!std::fs::read(temp.path().join("profiles/pending"))
        .unwrap()
        .windows(18)
        .any(|v| v == b"private-key-canary"));
    store.write_secret("pending", b"replacement", true).unwrap();
    assert_eq!(store.read_secret("pending").unwrap(), b"replacement");
}

#[test]
fn native_store_rejects_unprotected_directory_and_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unsafe");
    std::fs::create_dir(&path).unwrap();
    assert!(ProfileStore::open(path).is_err());
    let store = ProfileStore::open(temp.path().join("protected")).unwrap();
    store.write_secret("record", b"canary", false).unwrap();
    #[cfg(windows)]
    {
        std::fs::write(store.root().join("record"), b"corrupt DPAPI ciphertext").unwrap();
        assert!(store.read_secret("record").is_err());
    }
    let lock = store.lock("writer").unwrap();
    assert!(store.lock("writer").is_err());
    drop(lock);
    assert!(store.lock("writer").is_ok());
}
#[cfg(unix)]
#[test]
fn unix_store_rejects_symlinks_and_changed_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let store = ProfileStore::open(temp.path().join("profiles")).unwrap();
    store.write_secret("record", b"secret", false).unwrap();
    std::fs::set_permissions(
        store.root().join("record"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(store.read_secret("record").is_err());
    symlink(temp.path(), temp.path().join("link")).unwrap();
    assert!(ProfileStore::open(temp.path().join("link/profiles")).is_err());
}
