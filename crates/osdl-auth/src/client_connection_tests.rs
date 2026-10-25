use super::*;
use pb::authentication_server::{Authentication, AuthenticationServer};
use tonic::{Request, Response, Status};

struct Discovery(String);
#[tonic::async_trait]
impl Authentication for Discovery {
    async fn info(
        &self,
        _: Request<pb::AuthenticationInfoRequest>,
    ) -> std::result::Result<Response<pb::AuthenticationInfoResponse>, Status> {
        Ok(Response::new(pb::AuthenticationInfoResponse {
            mode: "public_key".into(),
            lab_id: self.0.clone(),
            instance: "test".into(),
            channel: "stable".into(),
            issuer: String::new(),
        }))
    }
    async fn renew(
        &self,
        _: Request<pb::AuthenticationRenewRequest>,
    ) -> std::result::Result<Response<pb::AuthenticationRenewResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
}
fn ca() -> (String, rcgen::Issuer<'static, rcgen::KeyPair>) {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::default();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    let pem = params.self_signed(&key).unwrap().pem();
    let issuer = rcgen::Issuer::from_ca_cert_pem(&pem, key).unwrap();
    (pem, issuer)
}
#[tokio::test]
async fn real_tls_failures_distinguish_transport_trust_and_discovery_binding() {
    crate::crypto_provider();
    let lab = uuid::Uuid::new_v4().to_string();
    let (pem, issuer) = ca();
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params =
        rcgen::CertificateParams::new(vec![format!("lab-{lab}.opensdl.invalid")]).unwrap();
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let certificate = params.signed_by(&key, &issuer).unwrap().pem();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .tls_config(
                tonic::transport::ServerTlsConfig::new()
                    .identity(Identity::from_pem(certificate, key.serialize_pem())),
            )
            .unwrap()
            .add_service(AuthenticationServer::new(Discovery(lab.clone())))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );
    let wrong_ca = ca().0;
    assert!(matches!(
        connect(&endpoint, tls(&lab, &wrong_ca)).await,
        Err(Error::Trust)
    ));
    let channel = connect(&endpoint, tls(&lab, &pem)).await.unwrap();
    assert!(info(channel.clone(), &lab, "test", Channel::Stable)
        .await
        .is_ok());
    assert!(matches!(
        info(channel, &lab, "other-instance", Channel::Stable).await,
        Err(Error::Trust)
    ));
    server.abort();
    let _ = server.await;
    assert!(matches!(
        connect(&endpoint, tls(&lab, &pem)).await,
        Err(Error::Transport)
    ));
}
#[test]
fn enrollment_failure_uses_status_type_and_never_remote_message() {
    assert_eq!(
        Error::enrollment(Status::unavailable("private-canary")),
        Error::Transport
    );
    assert_eq!(
        Error::enrollment(Status::unauthenticated("private-canary")),
        Error::Invitation
    );
    assert!(
        !Error::enrollment(Status::invalid_argument("private-canary"))
            .to_string()
            .contains("private-canary")
    );
}
