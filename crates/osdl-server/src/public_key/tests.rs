use super::*;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
async fn channel(
    address: std::net::SocketAddr,
    auth: &PublicKeyAuth,
    identity: Option<Identity>,
) -> tonic::transport::Channel {
    let mut tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(&auth.0.identity.ca))
        .domain_name(format!("lab-{}.opensdl.invalid", auth.lab_id()));
    if let Some(identity) = identity {
        tls = tls.identity(identity);
    }
    Endpoint::from_shared(format!("https://{address}"))
        .unwrap()
        .tls_config(tls)
        .unwrap()
        .connect()
        .await
        .unwrap()
}
#[tokio::test(flavor = "multi_thread")]
async fn real_tls_grant_scope_revoke_and_remote_admin_boundary() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &temp.path().join("auth"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    let engine = osdl_core::OsdlEngine::new(osdl_core::config::OsdlConfig::default(), vec![]);
    let service = crate::OsdlService::new(
        engine.handle(),
        crate::ServerIdentity {
            instance: "test".into(),
            version: "test".into(),
            pid: 0,
            started_at: crate::convert::now_ts(),
            socket_path: None,
            listen_addr: None,
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let server = tokio::spawn(crate::listeners::tcp(
        listener,
        service,
        crate::auth::ListenerAuth::PublicKey(auth.clone()),
        "test".into(),
        crate::listeners::TcpAccess::Remote,
        shutdown,
    ));
    let cold = std::time::Instant::now();
    let unauthenticated = channel(address, &auth, None).await;
    let cold_ms = cold.elapsed().as_millis();
    let mut anon = pb::osdl_client::OsdlClient::new(unauthenticated.clone());
    assert_eq!(
        anon.status(pb::StatusRequest {}).await.unwrap_err().code(),
        tonic::Code::Unauthenticated
    );
    let mut info = pb::authentication_client::AuthenticationClient::new(unauthenticated.clone());
    assert_eq!(
        info.info(pb::AuthenticationInfoRequest {})
            .await
            .unwrap()
            .into_inner()
            .lab_id,
        auth.lab_id()
    );
    let mut admin =
        pb::grant_administration_client::GrantAdministrationClient::new(unauthenticated.clone());
    assert_eq!(
        admin
            .list_grants(pb::ListGrantsRequest::default())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    let inv = auth.invite(Access::Read, None).unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::default();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let request = pb::PairRequest {
        invitation_id: inv.invitation_id,
        capability: inv.capability,
        csr_der: params.serialize_request(&key).unwrap().der().to_vec(),
        label: "client".into(),
    };
    let bytes = prost::Message::encoded_len(&request);
    let start = std::time::Instant::now();
    let response = pb::pairing_client::PairingClient::new(unauthenticated)
        .pair(request)
        .await
        .unwrap()
        .into_inner();
    let pair_ms = start.elapsed().as_millis();
    let identity = Identity::from_pem(response.certificate_pem, key.serialize_pem());
    let start = std::time::Instant::now();
    let authenticated = channel(address, &auth, Some(identity)).await;
    let warm_ms = start.elapsed().as_millis();
    let mut client = pb::osdl_client::OsdlClient::new(authenticated);
    assert!(client.status(pb::StatusRequest {}).await.is_ok());
    assert_eq!(
        client
            .send_command(pb::SendCommandRequest::default())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        client
            .shutdown(pb::ShutdownRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let grant_lease = auth
        .0
        .grants
        .read()
        .unwrap()
        .entries
        .values()
        .next()
        .unwrap()
        .lease
        .clone();
    let mut waiter_client = client.clone();
    let waiter = tokio::spawn(async move {
        waiter_client
            .wait_for_device(pb::WaitForDeviceRequest {
                selector: Some(pb::wait_for_device_request::Selector::DeviceId(
                    "absent".into(),
                )),
                timeout_ms: u32::MAX,
            })
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while grant_lease.is_drained() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("WaitForDevice must retain its authorization owner");
    let mut stream = client
        .stream_events(pb::StreamEventsRequest::default())
        .await
        .unwrap()
        .into_inner();
    let start = std::time::Instant::now();
    auth.revoke(&response.grant.unwrap().id).await.unwrap();
    let revoke_ms = start.elapsed().as_millis();
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error.metadata().get("opensdl-auth-error").unwrap(),
        "grant_revoked"
    );
    assert!(grant_lease.is_drained());
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), stream.message())
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(
        client
            .status(pb::StatusRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    println!("PERF tls_cold_ms={cold_ms} tls_mtls_ms={warm_ms} pair_ms={pair_ms} pair_request_payload_bytes={bytes} revoke_ms={revoke_ms}");
    // A correctly CA-signed certificate without a durable grant has no authority.
    let ungranted_key = rcgen::KeyPair::generate().unwrap();
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&ungranted_key)
        .unwrap();
    let (cert, _) = auth
        .0
        .identity
        .issue(csr.der(), &uuid::Uuid::new_v4().to_string(), now() + 300)
        .unwrap();
    let ungranted = channel(
        address,
        &auth,
        Some(Identity::from_pem(cert, ungranted_key.serialize_pem())),
    )
    .await;
    assert_eq!(
        pb::osdl_client::OsdlClient::new(ungranted)
            .status(pb::StatusRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    // Standard hostname validation is enforced independently of the pinned CA.
    assert!(Endpoint::from_shared(format!("https://{address}"))
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(&auth.0.identity.ca))
                .domain_name("wrong.opensdl.invalid")
        )
        .unwrap()
        .connect()
        .await
        .is_err());
    // An invalid presented client certificate cannot fall back to anonymous Info.
    let foreign = PublicKeyAuth::open(
        &temp.path().join("foreign"),
        "foreign",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    let foreign_key = rcgen::KeyPair::generate().unwrap();
    let foreign_csr = rcgen::CertificateParams::default()
        .serialize_request(&foreign_key)
        .unwrap();
    let (foreign_cert, _) = foreign
        .0
        .identity
        .issue(
            foreign_csr.der(),
            &uuid::Uuid::new_v4().to_string(),
            now() + 300,
        )
        .unwrap();
    let invalid = Endpoint::from_shared(format!("https://{address}"))
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(&auth.0.identity.ca))
                .domain_name(format!("lab-{}.opensdl.invalid", auth.lab_id()))
                .identity(Identity::from_pem(
                    foreign_cert,
                    foreign_key.serialize_pem(),
                )),
        )
        .unwrap()
        .connect()
        .await;
    if let Ok(channel) = invalid {
        assert!(
            pb::authentication_client::AuthenticationClient::new(channel)
                .info(pb::AuthenticationInfoRequest {})
                .await
                .is_err()
        );
    }
    // Certificate and grant expiry also fence a channel established before expiry.
    let inv = auth.invite(Access::Operate, Some(now() + 3)).unwrap();
    let expiry = inv.grant_expires_at.unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let response = auth
        .pair(&pb::PairRequest {
            invitation_id: inv.invitation_id,
            capability: inv.capability,
            csr_der: rcgen::CertificateParams::default()
                .serialize_request(&key)
                .unwrap()
                .der()
                .to_vec(),
            label: "expiring".into(),
        })
        .unwrap();
    let cert = response.certificate_pem;
    let identity = Identity::from_pem(&cert, key.serialize_pem());
    let authenticated = channel(address, &auth, Some(identity.clone())).await;
    let mut expiring = pb::osdl_client::OsdlClient::new(authenticated);
    let mut waiter_client = expiring.clone();
    let waiter = tokio::spawn(async move {
        waiter_client
            .wait_for_device(pb::WaitForDeviceRequest {
                selector: Some(pb::wait_for_device_request::Selector::DeviceId(
                    "absent".into(),
                )),
                timeout_ms: u32::MAX,
            })
            .await
    });
    let mut stream = expiring
        .stream_events(pb::StreamEventsRequest::default())
        .await
        .unwrap()
        .into_inner();
    tokio::time::sleep(std::time::Duration::from_secs(
        expiry.saturating_sub(now()) + 1,
    ))
    .await;
    assert_eq!(
        expiring
            .status(pb::StatusRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert!(stream.message().await.is_err());
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error.metadata().get("opensdl-auth-error").unwrap(),
        "grant_expired"
    );
    let expired_connect = Endpoint::from_shared(format!("https://{address}"))
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(&auth.0.identity.ca))
                .domain_name(format!("lab-{}.opensdl.invalid", auth.lab_id()))
                .identity(identity),
        )
        .unwrap()
        .connect()
        .await;
    if let Ok(channel) = expired_connect {
        assert!(pb::osdl_client::OsdlClient::new(channel)
            .status(pb::StatusRequest {})
            .await
            .is_err());
    }
    server.abort();
}
#[tokio::test]
async fn real_tls_wrong_ca_and_wrong_hostname_fail() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &temp.path().join("auth"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tls = auth.tls();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .tls_config(tls)
            .unwrap()
            .add_service(pb::pairing_server::PairingServer::new(auth))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let other = PublicKeyAuth::open(
        &temp.path().join("other"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    assert!(Endpoint::from_shared(format!("https://{addr}"))
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(&other.0.identity.ca))
                .domain_name(format!("lab-{}.opensdl.invalid", other.lab_id()))
        )
        .unwrap()
        .connect()
        .await
        .is_err());
    server.abort();
}

#[test]
fn invitation_expiry_and_bounded_records_and_corrupt_csr_fail_closed() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &temp.path().join("auth"),
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
    let len = csr.len();
    csr[len - 1] ^= 1;
    assert!(auth
        .pair(&pb::PairRequest {
            invitation_id: inv.invitation_id.clone(),
            capability: inv.capability.clone(),
            csr_der: csr,
            label: "test".into()
        })
        .is_err());
    auth.0
        .db
        .lock()
        .unwrap()
        .execute("UPDATE invitations SET expires=0", [])
        .unwrap();
    assert!(auth.describe(&inv.invitation_id, &inv.capability).is_err());
    for _ in 0..256 {
        auth.invite(Access::Read, None).unwrap();
    }
    assert!(auth.invite(Access::Read, None).is_err());
}
#[tokio::test]
async fn unpolled_grant_stream_revocation_releases_upstream_before_ack() {
    let lease = Lease::grant(now() + 300);
    let resource = lease.resource().unwrap();
    let (sent, done) = tokio::sync::oneshot::channel();
    struct Owner(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let owner = Owner(Some(sent));
    let upstream = async_stream::stream! {let _owner=owner;loop{yield Ok::<_,tonic::Status>(1u8);}};
    let mut body = crate::authorization::stream::guard_stream(Box::pin(upstream), Some(resource));
    tokio::task::yield_now().await;
    lease.revoke();
    tokio::time::timeout(std::time::Duration::from_secs(1), lease.drained())
        .await
        .unwrap();
    done.await.unwrap();
    use tokio_stream::StreamExt;
    assert!(body.next().await.unwrap().is_err());
    assert!(lease.resource().is_err());
}

#[test]
fn measured_grant_admission_scales_with_active_index_not_history() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &temp.path().join("auth"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    for size in [1usize, 100, 10_000] {
        {
            let mut index = auth.0.grants.write().unwrap();
            for id in 0..size {
                let leaf = id.to_le_bytes();
                let hash = Sha256::digest(leaf).to_vec();
                let grant = pb::Grant {
                    id: id.to_string(),
                    label: "benchmark".into(),
                    access: 2,
                    expires_at: None,
                    certificate_expires_at: (now() + 300) as i64,
                    revoked: false,
                };
                index.insert(
                    hash,
                    Arc::new(Authority {
                        grant,
                        lease: Lease::grant(now() + 300),
                    }),
                );
            }
        }
        let _database_unavailable = auth.0.db.lock().unwrap();
        let start = std::time::Instant::now();
        for index in 0..100_000usize {
            drop(
                auth.authorize_leaf(
                    &(index % size).to_le_bytes(),
                    crate::authorization::Permission::Read,
                )
                .unwrap(),
            );
        }
        println!("PERF grant_index={size} admissions=100000 ns_per_admission={} sql_queries=0 cloud_requests=0",start.elapsed().as_nanos()/100_000);
    }
}

#[tokio::test]
async fn concurrent_revoke_acknowledgments_both_wait_for_resource_cleanup() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let auth = PublicKeyAuth::open(
        &temp.path().join("auth"),
        "test",
        Channel::Stable,
        "https://100.64.0.9:50051",
    )
    .unwrap();
    let invitation = auth.invite(Access::Read, None).unwrap();
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap();
    let response = auth
        .pair(&pb::PairRequest {
            invitation_id: invitation.invitation_id,
            capability: invitation.capability,
            csr_der: csr.der().to_vec(),
            label: "test".into(),
        })
        .unwrap();
    let grant_id = response.grant.unwrap().id;
    let lease = auth
        .0
        .grants
        .read()
        .unwrap()
        .entries
        .values()
        .next()
        .unwrap()
        .lease
        .clone();
    let resource = lease.resource().unwrap();
    let first_auth = auth.clone();
    let first_id = grant_id.clone();
    let first = tokio::spawn(async move { first_auth.revoke(&first_id).await });
    lease.expired().await;
    let second_auth = auth.clone();
    let mut second = tokio::spawn(async move { second_auth.revoke(&grant_id).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut second)
            .await
            .is_err()
    );
    drop(resource);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
}
