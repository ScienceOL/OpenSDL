use osdl_auth::{ClientError, ProfileStore};
#[tokio::test]
async fn absent_or_corrupt_private_profile_is_persistence_not_identity_change() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let store = ProfileStore::open(temp.path().join("profiles")).unwrap();
    assert!(matches!(
        store.connect("missing").await,
        Err(ClientError::Persistence)
    ));
    store
        .write_secret("corrupt", b"invalid-json", false)
        .unwrap();
    assert!(matches!(
        store.metadata("corrupt"),
        Err(ClientError::Persistence)
    ));
}
