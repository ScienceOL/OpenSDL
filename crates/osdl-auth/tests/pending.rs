use osdl_auth::{Access, Channel, Invitation, ProfileStore};
fn invitation() -> Invitation {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::default();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    Invitation {
        lab_id: uuid::Uuid::new_v4().to_string(),
        instance: "test".into(),
        channel: Channel::Stable,
        endpoint: "https://100.64.0.9:50051".into(),
        ca_pem: params.self_signed(&key).unwrap().pem(),
        invitation_id: uuid::Uuid::new_v4().to_string(),
        capability: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
        expires_at: osdl_auth::now() + 600,
        access: Access::Read,
        grant_expires_at: None,
    }
}
#[test]
fn exact_pending_csr_survives_restart_and_trust_cannot_be_replaced() {
    let t = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let s = ProfileStore::open(t.path().join("profiles")).unwrap();
    let inv = invitation();
    let first = s.prepare_pair("test", &inv, false).unwrap();
    let second = s.prepare_pair("test", &inv, false).unwrap();
    assert_eq!(first.csr_der(), second.csr_der());
    let metadata = s.metadata("test").unwrap();
    assert_eq!(metadata.lab_id, inv.lab_id);
    assert_eq!(metadata.access, Access::Read);
    assert_eq!(metadata.grant_expires_at, None);
    assert!(matches!(metadata.state, osdl_auth::ProfileState::Pending));
    assert!(s.prepare_pair("test", &invitation(), false).is_err());
    assert!(Invitation::parse(&inv.encode().unwrap()).is_ok());
}

#[test]
fn same_replacement_invitation_keeps_pending_csr_but_new_invitation_rotates() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let store = ProfileStore::open(temp.path().join("profiles")).unwrap();
    let mut inv = invitation();
    let first = store.prepare_pair("test", &inv, true).unwrap();
    let saved = std::fs::read(store.root().join("test")).unwrap();
    let reopened = ProfileStore::open(store.root().to_owned()).unwrap();
    let started = std::time::Instant::now();
    let retry = reopened.prepare_pair("test", &inv, true).unwrap();
    assert_eq!(
        first.csr_der(),
        retry.csr_der(),
        "same replacement must retain its recovery key"
    );
    for _ in 0..20 {
        let repeated = reopened.prepare_pair("test", &inv, true).unwrap();
        assert_eq!(repeated.csr_der(), first.csr_der());
    }
    println!(
        "PERF same_replacement_21_retries_ms={} csr_bytes={}",
        started.elapsed().as_millis(),
        first.csr_der().len()
    );
    assert_eq!(std::fs::read(store.root().join("test")).unwrap(), saved);
    inv.invitation_id = uuid::Uuid::new_v4().to_string();
    let replaced = reopened.prepare_pair("test", &inv, true).unwrap();
    assert_ne!(first.csr_der(), replaced.csr_der());
    let saved = std::fs::read(store.root().join("test")).unwrap();
    inv.ca_pem = invitation().ca_pem;
    assert!(reopened.prepare_pair("test", &inv, true).is_err());
    assert_eq!(std::fs::read(store.root().join("test")).unwrap(), saved);
}
