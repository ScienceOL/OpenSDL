//! Local administration over a filesystem socket or protected loopback.
use super::{binding::valid_subject, members::Role, AccountAuth};
use crate::{
    authorization::lease::Lease, lockfile, InstanceRecord, LockfileGuard, Paths, SERVER_VERSION,
};
use anyhow::Context;
use osdl_proto::v1::{
    self as pb,
    account_membership_administration_server::{
        AccountMembershipAdministration, AccountMembershipAdministrationServer,
    },
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

impl AccountAuth {
    pub(crate) fn admin_service(&self) -> Option<AccountMembershipAdministrationServer<Self>> {
        self.members.as_ref()?;
        Some(
            AccountMembershipAdministrationServer::new(self.clone())
                .max_decoding_message_size(1024)
                .max_encoding_message_size(32768),
        )
    }

    fn validate_member(&self, subject: &str) -> Result<(), Status> {
        if !valid_subject(subject) {
            return Err(Status::invalid_argument(
                "provide an exact LiyanLabs account subject",
            ));
        }
        if subject == self.verifier.binding().account_id {
            return Err(Status::failed_precondition(
                "the bound laboratory owner cannot be changed here",
            ));
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl AccountMembershipAdministration for AccountAuth {
    async fn list_members(
        &self,
        request: Request<pb::ListAccountMembersRequest>,
    ) -> Result<Response<pb::ListAccountMembersResponse>, Status> {
        let after = request.into_inner().after_subject;
        if !after.is_empty() && !valid_subject(&after) {
            return Err(Status::invalid_argument("invalid membership cursor"));
        }
        let members = self.members.as_ref().ok_or_else(disabled)?;
        Ok(Response::new(
            members.list(&self.verifier.binding().account_id, &after),
        ))
    }

    async fn grant(
        &self,
        request: Request<pb::GrantAccountMemberRequest>,
    ) -> Result<Response<pb::AccountMember>, Status> {
        let pb::GrantAccountMemberRequest { subject, role } = request.into_inner();
        self.validate_member(&subject)?;
        let role = Role::from_wire(role)
            .ok_or_else(|| Status::invalid_argument("role must be viewer, operator, or admin"))?;
        let members = self.members.clone().ok_or_else(disabled)?;
        let (member, leases) = tokio::task::spawn_blocking(move || {
            let (member, leases) = members.grant(subject, role)?;
            log::info!(target: "opensdl::account_audit", "operation=grant_membership actor=local_administrator subject={:?} role={role:?}", member.subject);
            Ok((member, leases))
        })
        .await
        .map_err(store_error)?
        .map_err(|error: anyhow::Error| store_error(error))?;
        drain(leases).await;
        Ok(Response::new(member))
    }

    async fn revoke(
        &self,
        request: Request<pb::RevokeAccountMemberRequest>,
    ) -> Result<Response<pb::RevokeAccountMemberResponse>, Status> {
        let subject = request.into_inner().subject;
        self.validate_member(&subject)?;
        let members = self.members.clone().ok_or_else(disabled)?;
        let leases = tokio::task::spawn_blocking(move || {
            let leases = members.revoke(&subject)?;
            if leases.is_some() {
                log::info!(target: "opensdl::account_audit", "operation=revoke_membership actor=local_administrator subject={subject:?}");
            }
            Ok(leases)
        })
        .await
        .map_err(store_error)?
        .map_err(|error: anyhow::Error| store_error(error))?
        .ok_or_else(|| Status::not_found("account is not a member of this laboratory"))?;
        drain(leases).await;
        Ok(Response::new(pb::RevokeAccountMemberResponse {}))
    }
}

fn disabled() -> Status {
    Status::failed_precondition("account membership administration is unavailable")
}

fn store_error(error: impl std::fmt::Display) -> Status {
    log::error!("account membership store: {error}");
    Status::internal("account membership update failed")
}

async fn drain(leases: Vec<Arc<Lease>>) {
    for lease in leases {
        lease.drained().await;
    }
}

/// Same-user administration for account-enabled services that have no local
/// engine listener, such as a cluster control-plane gateway.
pub struct AccountAdminEndpoint {
    service: AccountMembershipAdministrationServer<AccountAuth>,
    #[cfg(unix)]
    socket: (tokio::net::UnixListener, SocketCleanup),
    #[cfg(windows)]
    loopback: (tokio::net::TcpListener, String),
    _discovery: LockfileGuard,
}

#[cfg(unix)]
struct SocketCleanup(std::path::PathBuf);
#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl AccountAdminEndpoint {
    pub async fn bind(account: &AccountAuth, paths: &Paths) -> anyhow::Result<Self> {
        let instance = &account.verifier.binding().instance;
        let service = account
            .admin_service()
            .context("account memberships are not configured")?;
        #[cfg(unix)]
        let path = paths.default_socket_path(instance);
        #[cfg(unix)]
        let (socket_path, listen_addr) = (Some(path.clone()), None);
        #[cfg(windows)]
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        #[cfg(windows)]
        let address = listener.local_addr()?;
        #[cfg(windows)]
        let (socket_path, listen_addr) = (None, Some(address.to_string()));

        let locks = paths.lock_dir();
        let discovery = lockfile::write(
            &locks,
            &InstanceRecord::new(
                instance,
                std::process::id(),
                SERVER_VERSION,
                socket_path,
                listen_addr,
            ),
        )
        .map_err(anyhow::Error::msg)?;

        #[cfg(unix)]
        let socket = (
            crate::listeners::bind_unix_socket(&path)?,
            SocketCleanup(path),
        );
        #[cfg(windows)]
        let loopback = {
            let access = crate::local_access::LocalAccess::generate(format!("http://{address}"))?;
            access.save(&locks, instance)?;
            (listener, access.token)
        };
        Ok(Self {
            service,
            _discovery: discovery,
            #[cfg(unix)]
            socket,
            #[cfg(windows)]
            loopback,
        })
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        let mut server = tonic::transport::Server::builder();
        #[cfg(unix)]
        {
            server
                .add_service(self.service)
                .serve_with_incoming(tokio_stream::wrappers::UnixListenerStream::new(
                    self.socket.0,
                ))
                .await?;
        }
        #[cfg(windows)]
        {
            let (listener, token) = self.loopback;
            server
                .add_service(tonic::service::interceptor::InterceptedService::new(
                    self.service,
                    crate::auth::bearer_interceptor(token),
                ))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await?;
        }
        Ok(())
    }
}
