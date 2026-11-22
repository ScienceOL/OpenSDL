use super::*;

fn update(name: &str, description: &str, revision: &str) -> pb::UpdateLabProfileRequest {
    pb::UpdateLabProfileRequest {
        name: name.into(),
        description: description.into(),
        revision: revision.into(),
    }
}

#[tokio::test]
async fn concurrent_writes_allow_one_revision_then_reload_the_winner() {
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(data.path(), "optics", Some("Optics Lab")).unwrap());
    let original = store.snapshot(true);
    let other = Store::open(data.path(), "another", Some("Optics Lab"))
        .unwrap()
        .snapshot(true);
    assert_ne!(original.revision, other.revision);
    assert_eq!(
        store
            .clone()
            .update(update("Wrong lab", "", &other.revision), None)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Aborted
    );
    let (first, second) = tokio::join!(
        store.clone().update(
            update(" Optics North ", "Imaging\nand spectra", &original.revision),
            None
        ),
        store.clone().update(
            update("Optics South", "Microscopy", &original.revision),
            None
        ),
    );
    let winner = match (first, second) {
        (Ok(saved), Err(error)) | (Err(error), Ok(saved)) => {
            assert_eq!(error.code(), tonic::Code::Aborted);
            saved
        }
        _ => panic!("exactly one competing writer must succeed"),
    };
    assert_ne!(winner.revision, original.revision);
    assert_eq!(store.snapshot(true), winner);
    drop(store);

    let restarted = Arc::new(Store::open(data.path(), "optics", Some("New startup name")).unwrap());
    assert_eq!(restarted.snapshot(true), winner);
    assert_eq!(
        restarted
            .clone()
            .update(
                update(&winner.name, &winner.description, &winner.revision),
                None
            )
            .await
            .unwrap(),
        winner
    );
    let replaced = restarted
        .clone()
        .update(
            update(" Optics East ", "Laser\nimaging", &winner.revision),
            None,
        )
        .await
        .unwrap();
    assert_eq!(replaced.name, "Optics East");
    assert_eq!(replaced.description, "Laser\nimaging");
    assert_ne!(replaced.revision, winner.revision);
    drop(restarted);
    assert_eq!(
        Store::open(data.path(), "optics", None)
            .unwrap()
            .snapshot(true),
        replaced
    );
}

#[tokio::test]
async fn text_limits_count_unicode_characters_and_reject_control_characters() {
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(data.path(), "optics", Some("Optics Lab")).unwrap());
    let original = store.snapshot(true);
    for (name, description) in [
        (" ".to_owned(), String::new()),
        ("Lab\u{2028}name".to_owned(), String::new()),
        ("镜".repeat(129), String::new()),
        ("Lab".to_owned(), "镜".repeat(501)),
        ("Lab".to_owned(), "bad\0description".to_owned()),
    ] {
        let error = store
            .clone()
            .update(update(&name, &description, &original.revision), None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    assert_eq!(store.snapshot(true), original);
    let name = "镜".repeat(128);
    let description = format!("{}\n", "镜".repeat(499));
    let saved = store
        .update(update(&name, &description, &original.revision), None)
        .await
        .unwrap();
    assert_eq!(saved.name, name);
    assert_eq!(saved.description, description);
}

#[tokio::test]
async fn failure_before_publication_keeps_the_cached_revision() {
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(data.path(), "optics", Some("Optics Lab")).unwrap());
    let original = store.snapshot(true);
    std::fs::create_dir(&store.path).unwrap();
    let error = store
        .clone()
        .update(update("Changed", "", &original.revision), None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Internal);
    assert_eq!(store.snapshot(true), original);
}

#[test]
fn one_live_owner_per_profile_even_for_an_aliased_directory() {
    let data = tempfile::tempdir().unwrap();
    let live = Store::open(data.path(), "optics", Some("Live")).unwrap();
    assert!(Store::open(&data.path().join("."), "optics", Some("Other")).is_err());
    drop(live);
    assert!(Store::open(data.path(), "optics", Some("Restarted")).is_ok());
}

#[tokio::test]
async fn differently_cased_instances_keep_separate_saved_profiles() {
    let data = tempfile::tempdir().unwrap();
    let upper = Arc::new(Store::open(data.path(), "OPTICS", Some("Upper")).unwrap());
    let initial = upper.snapshot(true);
    upper
        .update(update("Upper saved", "", &initial.revision), None)
        .await
        .unwrap();
    let lower = Store::open(data.path(), "optics", Some("Lower")).unwrap();
    assert_eq!(lower.snapshot(true).name, "Lower");
}

#[test]
fn derived_name_keeps_the_host_prefix_within_the_display_limit() {
    assert_eq!(
        hostname_display_name(&"镜".repeat(200), "instrument"),
        "镜".repeat(128)
    );
    assert_eq!(
        hostname_display_name("host.example", "instrument"),
        "host.example"
    );
    assert_eq!(hostname_display_name("", "instrument"), "instrument");
}

#[tokio::test]
async fn a_queued_edit_holds_its_resource_and_checks_its_authority_before_publication() {
    use crate::authorization::lease::Lease;

    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(data.path(), "optics", Some("Optics Lab")).unwrap());
    let original = store.snapshot(true);
    let writer = store.writer.lock().await;
    let lease = Lease::grant(osdl_auth::now() + 300);
    let edit = store.clone().update(
        update("Delayed", "", &original.revision),
        Some(lease.resource().unwrap()),
    );
    tokio::pin!(edit);
    tokio::select! {
        biased;
        _ = &mut edit => panic!("edit finished while the writer was held"),
        _ = async {} => {}
    }
    assert!(!lease.is_drained());
    assert!(lease.revoke());
    drop(writer);
    let error = edit.await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unauthenticated);
    assert_eq!(store.snapshot(true), original);
    assert!(lease.is_drained());
}
