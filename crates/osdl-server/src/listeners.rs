//! Listener composition is the authority boundary; admin RPCs exist only locally.
use crate::{
    auth::{bearer_interceptor, ListenerAuth},
    authorization::info::AuthenticationService,
    authorization::Authorizer,
    OsdlService,
};
use osdl_proto::v1 as pb;
use std::sync::Arc;
use tokio::sync::Notify;
pub(crate) enum TcpAccess {
    Remote,
    LocalCapability(String),
}
fn info(policy: &ListenerAuth, instance: &str) -> AuthenticationService {
    AuthenticationService {
        policy: policy.clone(),
        instance: instance.into(),
    }
}
pub(crate) async fn tcp(
    listener: tokio::net::TcpListener,
    service: OsdlService,
    policy: ListenerAuth,
    instance: String,
    access: TcpAccess,
    shutdown: Arc<Notify>,
) -> Result<(), tonic::transport::Error> {
    let key = match &policy {
        ListenerAuth::PublicKey(a) => Some(a.clone()),
        _ => None,
    };
    let local = matches!(&access, TcpAccess::LocalCapability(_));
    let token = match access {
        TcpAccess::LocalCapability(token) => Some(token),
        TcpAccess::Remote => match &policy {
            ListenerAuth::Bearer(token) => Some(token.clone()),
            _ => None,
        },
    };
    let authorizer = if local {
        Authorizer::Local
    } else {
        match &policy {
            ListenerAuth::Account(a) => Authorizer::Account(a.clone()),
            ListenerAuth::PublicKey(a) => Authorizer::PublicKey(a.clone()),
            _ => Authorizer::Local,
        }
    };
    let service = pb::osdl_server::OsdlServer::new(service.with_authorizer(authorizer));
    let interceptor = move |req| {
        if let Some(ref token) = token {
            tonic::service::Interceptor::call(&mut bearer_interceptor(token.clone()), req)
        } else {
            Ok(req)
        }
    };
    let operations =
        tonic::service::interceptor::InterceptedService::new(service, interceptor.clone());
    let grants = if local {
        key.clone().map(|auth| {
            tonic::service::interceptor::InterceptedService::new(
                pb::grant_administration_server::GrantAdministrationServer::new(auth)
                    .max_decoding_message_size(1024)
                    .max_encoding_message_size(32768),
                interceptor,
            )
        })
    } else {
        None
    };
    let pairing = if local {
        None
    } else {
        key.clone().map(|auth| {
            pb::pairing_server::PairingServer::new(auth)
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192)
        })
    };
    let mut builder = tonic::transport::Server::builder();
    if !local {
        if let Some(key) = key {
            builder = builder.tls_config(key.tls())?;
        }
    }
    builder
        .add_service(
            pb::authentication_server::AuthenticationServer::new(info(&policy, &instance))
                .max_decoding_message_size(1024),
        )
        .add_service(operations)
        .add_optional_service(pairing)
        .add_optional_service(grants)
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            async move { shutdown.notified().await },
        )
        .await
}
#[cfg(unix)]
pub(crate) async fn unix(
    listener: tokio::net::UnixListener,
    service: OsdlService,
    policy: ListenerAuth,
    shutdown: Arc<Notify>,
) -> Result<(), tonic::transport::Error> {
    let grants = match policy {
        ListenerAuth::PublicKey(auth) => Some(
            pb::grant_administration_server::GrantAdministrationServer::new(auth)
                .max_decoding_message_size(1024)
                .max_encoding_message_size(32768),
        ),
        _ => None,
    };
    tonic::transport::Server::builder()
        .add_service(pb::osdl_server::OsdlServer::new(service))
        .add_optional_service(grants)
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::UnixListenerStream::new(listener),
            async move { shutdown.notified().await },
        )
        .await
}
