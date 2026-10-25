use osdl_auth::{Access, Channel};
use osdl_server::public_key::PublicKeyAuth;
#[tokio::test]
async fn durable_identity_one_use_exact_retry_and_revoke() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let path = root.path().join("auth");
    let auth =
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").unwrap();
    let inv = auth.invite(Access::Read, None).unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&key)
        .unwrap()
        .der()
        .to_vec();
    let request = osdl_proto::v1::PairRequest {
        invitation_id: inv.invitation_id.clone(),
        capability: inv.capability.clone(),
        csr_der: csr,
        label: "test".into(),
    };
    let response = auth.pair(&request).unwrap();
    assert_eq!(response.grant.as_ref().unwrap().access, 1);
    assert_eq!(
        auth.pair(&request).unwrap().certificate_pem,
        response.certificate_pem
    );
    let mut other = request.clone();
    other.csr_der = rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec();
    assert!(auth.pair(&other).is_err());
    auth.revoke(&response.grant.unwrap().id).await.unwrap();
    drop(auth);
    let auth =
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").unwrap();
    assert_eq!(auth.lab_id(), inv.lab_id);
    assert!(auth.list("", 100).unwrap().grants[0].revoked);
}

#[test]
fn csr_der_with_unsigned_trailing_data_is_rejected() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &root.path().join("auth"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    let inv = auth.invite(Access::Read, None).unwrap();
    let mut csr = rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec();
    csr.extend_from_slice(b"unsigned");
    assert!(auth
        .pair(&osdl_proto::v1::PairRequest {
            invitation_id: inv.invitation_id,
            capability: inv.capability,
            csr_der: csr,
            label: "test".into()
        })
        .is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_exact_retry_creates_one_grant() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &root.path().join("auth"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    let inv = auth.invite(Access::Operate, None).unwrap();
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec();
    let request = osdl_proto::v1::PairRequest {
        invitation_id: inv.invitation_id,
        capability: inv.capability,
        csr_der: csr,
        label: "test".into(),
    };
    let start = std::time::Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let a = auth.clone();
        let r = request.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            a.pair(&r).unwrap().grant.unwrap().id
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        ids.insert(task.await.unwrap());
    }
    assert_eq!(ids.len(), 1);
    assert_eq!(auth.list("", 100).unwrap().grants.len(), 1);
    println!(
        "PERF concurrent_32_pair_retry_ms={}",
        start.elapsed().as_millis()
    );
}

#[test]
fn missing_identity_or_auth_database_is_a_recovery_error() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let path = root.path().join("auth");
    let auth =
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").unwrap();
    assert!(
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").is_err()
    );
    drop(auth);
    std::fs::rename(path.join("identity"), path.join("identity-backup")).unwrap();
    assert!(
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").is_err()
    );
    std::fs::rename(path.join("identity-backup"), path.join("identity")).unwrap();
    std::fs::rename(path.join("grants.sqlite"), path.join("grants-backup")).unwrap();
    assert!(
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").is_err()
    );
}

#[test]
fn mismatched_persisted_ca_key_is_a_recovery_error() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let path = temp.path().join("auth");
    drop(PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").unwrap());
    let store = osdl_auth::ProfileStore::open(path.clone()).unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&store.read_secret("identity").unwrap()).unwrap();
    record["ca_key"] =
        serde_json::Value::String(rcgen::KeyPair::generate().unwrap().serialize_pem());
    store
        .write_secret("identity", &serde_json::to_vec(&record).unwrap(), true)
        .unwrap();
    assert!(
        PublicKeyAuth::open(&path, "test", Channel::Stable, "https://100.64.0.9:50051").is_err()
    );
}
