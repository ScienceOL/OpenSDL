use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;
fn fixture() -> (Binding, Ed25519KeyPair) {
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    (
        Binding {
            issuer: "https://liyanlabs.local".into(),
            channel: "dev".into(),
            account_id: "account-owner".into(),
            lab_id: "11111111-1111-4111-8111-111111111111".into(),
            instance: "laboratory".into(),
            key_id: "dev-key".into(),
            public_key: B64.encode(key.public_key().as_ref()),
        },
        key,
    )
}
fn claims() -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    json!({
        "iss":"https://liyanlabs.local",
        "sub":"account-owner",
        "aud":"11111111-1111-4111-8111-111111111111",
        "channel":"dev",
        "device_id":"22222222-2222-4222-8222-222222222222",
        "device_generation":3,
        "installation_id":"33333333-3333-4333-8333-333333333333",
        "scope":"opensdl:read opensdl:operate",
        "iat":now,
        "exp":now+300,
    })
}
fn token(key: &Ed25519KeyPair, claims: serde_json::Value) -> String {
    let data = format!(
        "{}.{}",
        B64.encode(br#"{"alg":"EdDSA","typ":"liyanlabs.opensdl.identity+jwt","kid":"dev-key"}"#),
        B64.encode(serde_json::to_vec(&claims).unwrap())
    );
    format!(
        "{}.{}",
        data,
        B64.encode(key.sign(data.as_bytes()).as_ref())
    )
}
#[test]
fn real_ed25519_identity_is_accepted() {
    let (binding, key) = fixture();
    let verifier = Verifier::new(binding).unwrap();
    assert!(verifier.verify(&token(&key, claims())).is_ok());
}
#[test]
fn canonical_binding_rejects_aliases_and_cross_channel() {
    let (binding, _) = fixture();
    assert!(binding.validate().is_ok());
    for issuer in [
        "https://liyanlabs.local/",
        "https://sciol.ac.cn",
        "http://liyanlabs.local",
    ] {
        let mut b = binding.clone();
        b.issuer = issuer.into();
        assert!(b.validate().is_err());
    }
    let mut b = binding;
    b.channel = "stable".into();
    assert!(b.validate().is_err());
}
#[test]
fn foreign_or_expired_claims_never_authorize() {
    let (binding, key) = fixture();
    let verifier = Verifier::new(binding).unwrap();
    for (field, value) in [
        ("sub", json!("other")),
        ("aud", json!("44444444-4444-4444-8444-444444444444")),
        ("iss", json!("https://liyanlabs.com")),
        ("channel", json!("stable")),
        ("scope", json!("opensdl:manage")),
        ("device_id", json!("bad")),
        ("device_generation", json!(0)),
        (
            "installation_id",
            json!("00000000-0000-0000-0000-000000000000"),
        ),
        ("exp", json!(1)),
        ("exp", json!(u64::MAX)),
        ("iat", json!(u64::MAX)),
    ] {
        let mut c = claims();
        c[field] = value;
        assert!(verifier.verify(&token(&key, c)).is_err(), "{field}");
    }
    let other = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
    assert!(verifier.verify(&token(&other, claims())).is_err());
}

async fn grpc_fixture() -> (
    pb::osdl_client::OsdlClient<tonic::transport::Channel>,
    pb::authentication_client::AuthenticationClient<tonic::transport::Channel>,
    tokio::task::JoinHandle<()>,
    Ed25519KeyPair,
) {
    let (binding, key) = fixture();
    let account = AccountAuth::new(binding).unwrap();
    let engine = osdl_core::OsdlEngine::new(osdl_core::config::OsdlConfig::default(), vec![]);
    let identity = crate::ServerIdentity {
        instance: "laboratory".into(),
        version: "test".into(),
        pid: 0,
        started_at: crate::convert::now_ts(),
        socket_path: None,
        listen_addr: None,
    };
    let service = crate::OsdlService::new(engine.handle(), identity).with_account(account.clone());
    let auth = super::AuthenticationService {
        account: Some(account),
        mode: "account".into(),
        instance: "laboratory".into(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(pb::osdl_server::OsdlServer::new(service))
            .add_service(pb::authentication_server::AuthenticationServer::new(auth))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    (
        pb::osdl_client::OsdlClient::new(channel.clone()),
        pb::authentication_client::AuthenticationClient::new(channel),
        task,
        key,
    )
}
use osdl_proto::v1 as pb;
fn authorized<T>(body: T, key: &Ed25519KeyPair, c: serde_json::Value) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", token(key, c)).parse().unwrap(),
    );
    req
}
#[tokio::test]
async fn grpc_account_boundary_and_scope() {
    let (mut client, mut auth, task, key) = grpc_fixture().await;
    assert_eq!(
        auth.info(pb::AuthenticationInfoRequest {})
            .await
            .unwrap()
            .into_inner()
            .mode,
        "account"
    );
    assert_eq!(
        client
            .status(pb::StatusRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    assert!(client
        .status(authorized(pb::StatusRequest {}, &key, claims()))
        .await
        .is_ok());
    let mut read = claims();
    read["scope"] = json!("opensdl:read");
    assert_eq!(
        client
            .send_command(authorized(pb::SendCommandRequest::default(), &key, read))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        client
            .shutdown(authorized(pb::ShutdownRequest {}, &key, claims()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    for field in ["sub", "aud", "iss", "channel"] {
        let mut c = claims();
        c[field] = json!("wrong");
        assert_eq!(
            client
                .status(authorized(pb::StatusRequest {}, &key, c))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
    }
    task.abort();
}
#[tokio::test]
async fn grpc_renew_keeps_existing_stream_then_expires() {
    let (mut client, mut auth, task, key) = grpc_fixture().await;
    let mut c = claims();
    c["exp"] = json!(c["iat"].as_u64().unwrap() + 2);
    let mut stream = client
        .stream_events(authorized(
            pb::StreamEventsRequest::default(),
            &key,
            c.clone(),
        ))
        .await
        .unwrap()
        .into_inner();
    c["exp"] = json!(c["iat"].as_u64().unwrap() + 4);
    assert_eq!(
        auth.renew(authorized(
            pb::AuthenticationRenewRequest {},
            &key,
            c.clone()
        ))
        .await
        .unwrap()
        .into_inner()
        .expires_at,
        c["exp"].as_i64().unwrap()
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(2100), stream.message())
            .await
            .is_err()
    );
    let expired = tokio::time::timeout(std::time::Duration::from_secs(2), stream.message())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(expired.code(), tonic::Code::Unauthenticated);
    assert_eq!(
        expired
            .metadata()
            .get("opensdl-auth-error")
            .and_then(|v| v.to_str().ok()),
        Some("authorization_expired")
    );
    task.abort();
}

#[test]
fn binding_is_atomic_and_existing_identity_cannot_be_replaced() {
    let (binding, _) = fixture();
    let dir = tempfile::tempdir().unwrap();
    binding.save_new(dir.path()).unwrap();
    assert_eq!(Binding::load(dir.path(), "laboratory").unwrap(), binding);
    let mut replacement = binding.clone();
    replacement.lab_id = uuid::Uuid::new_v4().to_string();
    assert!(replacement.save_new(dir.path()).is_err());
    assert_eq!(Binding::load(dir.path(), "laboratory").unwrap(), binding);
    assert!(Binding::path(dir.path(), "../escape").is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(Binding::path(dir.path(), "laboratory").unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn account_listener_is_bound_to_instance_and_private_network() {
    let (binding, _) = fixture();
    let auth = crate::auth::ListenerAuth::Account(AccountAuth::new(binding).unwrap());
    for addr in [
        "127.0.0.1:50051",
        "[::1]:50051",
        "100.64.0.9:50051",
        "100.127.255.254:50051",
    ] {
        assert!(auth
            .validate_listener("laboratory", Some(addr.parse().unwrap()))
            .is_ok());
    }
    for addr in [
        "0.0.0.0:50051",
        "[::]:50051",
        "8.8.8.8:50051",
        "192.168.1.2:50051",
        "100.128.0.1:50051",
    ] {
        assert!(
            auth.validate_listener("laboratory", Some(addr.parse().unwrap()))
                .is_err(),
            "{addr}"
        );
    }
    assert!(auth.validate_listener("different", None).is_err());
}

#[test]
fn jwt_header_and_signature_are_closed_and_errors_do_not_echo_tokens() {
    let (binding, key) = fixture();
    let verifier = Verifier::new(binding).unwrap();
    for (field, value) in [("alg", "HS256"), ("typ", "JWT"), ("kid", "foreign-key")] {
        let mut header =
            json!({"alg":"EdDSA","typ":"liyanlabs.opensdl.identity+jwt","kid":"dev-key"});
        header[field] = json!(value);
        let data = format!(
            "{}.{}",
            B64.encode(serde_json::to_vec(&header).unwrap()),
            B64.encode(serde_json::to_vec(&claims()).unwrap())
        );
        let token = format!(
            "{}.{}",
            data,
            B64.encode(key.sign(data.as_bytes()).as_ref())
        );
        let error = verifier.verify(&token).unwrap_err().to_string();
        assert!(!error.contains(&token));
    }
}
#[tokio::test]
async fn measure_local_verification_and_concurrent_rpc_costs() {
    let (binding, key) = fixture();
    let verifier = Verifier::new(binding).unwrap();
    let jwt = token(&key, claims());
    let start = std::time::Instant::now();
    for _ in 0..2000 {
        verifier.verify(&jwt).unwrap();
    }
    let verify = start.elapsed();
    let start = std::time::Instant::now();
    let (client, mut auth, task, key) = grpc_fixture().await;
    let setup = start.elapsed();
    let jwt = token(&key, claims());
    let start = std::time::Instant::now();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let mut client = client.clone();
        let jwt = jwt.clone();
        workers.spawn(async move {
            for _ in 0..125 {
                let mut request = tonic::Request::new(pb::StatusRequest {});
                request
                    .metadata_mut()
                    .insert("authorization", format!("Bearer {jwt}").parse().unwrap());
                client.status(request).await.unwrap();
            }
        });
    }
    while let Some(result) = workers.join_next().await {
        result.unwrap();
    }
    let rpc = start.elapsed();
    let start = std::time::Instant::now();
    let renewal = auth
        .renew(authorized(
            pb::AuthenticationRenewRequest {},
            &key,
            claims(),
        ))
        .await
        .unwrap();
    println!("PERF token_bytes={} verification_2000_us={} setup_us={} concurrent_1000_rpc_us={} renew_us={} renewal_payload_bytes={}",jwt.len(),verify.as_micros(),setup.as_micros(),rpc.as_micros(),start.elapsed().as_micros(),prost::Message::encoded_len(renewal.get_ref()));
    task.abort();
}

#[tokio::test]
async fn grpc_auth_error_metadata_is_safe_and_verified_before_classification() {
    let (mut client, _, task, key) = grpc_fixture().await;
    let mut other = claims();
    other["sub"] = json!("another-owner");
    let status = client
        .status(authorized(pb::StatusRequest {}, &key, other.clone()))
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::Unauthenticated);
    assert_eq!(
        status
            .metadata()
            .get("opensdl-auth-error")
            .and_then(|v| v.to_str().ok()),
        Some("account_mismatch")
    );
    let foreign_key = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
    let status = client
        .status(authorized(pb::StatusRequest {}, &foreign_key, other))
        .await
        .unwrap_err();
    assert_eq!(
        status
            .metadata()
            .get("opensdl-auth-error")
            .and_then(|v| v.to_str().ok()),
        Some("invalid_assertion")
    );
    let mut expired = claims();
    let now = expired["iat"].as_u64().unwrap();
    expired["iat"] = json!(now - 10);
    expired["exp"] = json!(now - 1);
    let status = client
        .status(authorized(pb::StatusRequest {}, &key, expired))
        .await
        .unwrap_err();
    assert_eq!(
        status
            .metadata()
            .get("opensdl-auth-error")
            .and_then(|v| v.to_str().ok()),
        Some("authorization_expired")
    );
    task.abort();
}

#[test]
fn concurrent_binding_publication_has_exactly_one_winner() {
    let (binding, _) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let mut binding = binding.clone();
                binding.lab_id = uuid::Uuid::new_v4().to_string();
                let path = directory.path();
                scope.spawn(move || {
                    let saved = binding.save_new(path).is_ok();
                    (saved, binding)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let winners: Vec<_> = results.into_iter().filter(|(saved, _)| *saved).collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(
        Binding::load(directory.path(), "laboratory").unwrap(),
        winners[0].1
    );
}
