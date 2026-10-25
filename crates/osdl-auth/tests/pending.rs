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
