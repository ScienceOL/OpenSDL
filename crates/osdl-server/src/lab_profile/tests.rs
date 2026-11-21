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
            .update(update("Wrong lab", "", &other.revision))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Aborted
    );
    let (first, second) = tokio::join!(
        store.clone().update(update(
            " Optics North ",
            "Imaging\nand spectra",
            &original.revision
        )),
        store
            .clone()
            .update(update("Optics South", "Microscopy", &original.revision)),
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
            .update(update(&winner.name, &winner.description, &winner.revision))
            .await
            .unwrap(),
        winner
    );
    let replaced = restarted
        .clone()
        .update(update(" Optics East ", "Laser\nimaging", &winner.revision))
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
            .update(update(&name, &description, &original.revision))
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    assert_eq!(store.snapshot(true), original);
    let name = "镜".repeat(128);
    let description = format!("{}\n", "镜".repeat(499));
    let saved = store
        .update(update(&name, &description, &original.revision))
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
    std::fs::create_dir(data.path().join("optics.lab-profile.json")).unwrap();
    let error = store
        .clone()
        .update(update("Changed", "", &original.revision))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Internal);
    assert_eq!(store.snapshot(true), original);
}
