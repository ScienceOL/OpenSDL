use super::{
    tests::{authorized, claims, fixture},
    AccountAdminEndpoint, AccountAuth,
};
use crate::{
    authorization::{Authorizer, Permission},
    Paths, ServerIdentity,
};
use osdl_proto::v1::{
    self as pb,
    account_membership_administration_client::AccountMembershipAdministrationClient as AdminClient,
};
use ring::signature::Ed25519KeyPair;
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tonic::{
    transport::{Channel, Endpoint},
    Code, Request, Status,
};

const VIEWER: &str = "viewer-subject";
const OPERATOR: &str = "operator-subject";
const ADMIN: &str = "admin-subject";

fn member_claims(subject: &str, scope: &str) -> Value {
    let mut c = claims();
    c["sub"] = json!(subject);
    c["scope"] = json!(scope);
    c
}

pub(super) fn bound_account() -> (TempDir, Paths, AccountAuth, Ed25519KeyPair) {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let (binding, key) = fixture();
    let runtime_dir = osdl_auth::ProfileStore::open(root.path().join("run"))
        .unwrap()
        .root()
        .to_owned();
    let paths = Paths {
        config_dir: root.path().join("config"),
        state_dir: root.path().join("state"),
        cache_dir: root.path().join("cache"),
        runtime_dir,
    };
    std::fs::create_dir(&paths.state_dir).unwrap();
    binding.save_new(&paths.config_dir).unwrap();
    let auth = AccountAuth::open(&paths.config_dir, &binding.instance).unwrap();
    (root, paths, auth, key)
}

struct Gateway {
    _root: TempDir,
    pub key: Ed25519KeyPair,
    pub external: Channel,
    administration: AdminClient<Channel>,
    local_bearer: Option<String>,
    tasks: [JoinHandle<()>; 2],
}

impl Gateway {
    async fn start() -> Self {
        let (root, paths, auth, key) = bound_account();
        let admin = AccountAdminEndpoint::bind(&auth, &paths).await.unwrap();
        let administration = tokio::spawn(async move { admin.serve().await.unwrap() });
        #[cfg(unix)]
        let (local, local_bearer) = {
            let socket = paths.default_socket_path("laboratory");
            let local = Endpoint::from_static("http://[::]")
                .connect_with_connector(tower::service_fn(move |_| {
                    let socket = socket.clone();
                    async move {
                        tokio::net::UnixStream::connect(socket)
                            .await
                            .map(hyper_util::rt::TokioIo::new)
                    }
                }))
                .await
                .unwrap();
            (local, None)
        };
        #[cfg(windows)]
        let (local, local_bearer) = {
            let access =
                crate::local_access::LocalAccess::load(&paths.lock_dir(), "laboratory").unwrap();
            let local = Endpoint::from_shared(access.endpoint)
                .unwrap()
                .connect()
                .await
                .unwrap();
            (local, Some(format!("Bearer {}", access.token)))
        };
        let identity = ServerIdentity {
            instance: "laboratory".into(),
            version: "test".into(),
            pid: 0,
            started_at: crate::convert::now_ts(),
            socket_path: None,
            listen_addr: None,
        };
        let engine = osdl_core::OsdlEngine::new(osdl_core::config::OsdlConfig::default(), vec![]);
        let upstream = crate::OsdlService::new(engine.handle(), identity);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = tokio::spawn(async move {
            crate::serve_account_endpoint(listener, auth, upstream, None, &paths.state_dir)
                .await
                .unwrap();
        });
        let external = Endpoint::from_shared(format!("http://{address}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        Self {
            _root: root,
            key,
            external,
            administration: AdminClient::new(local),
            local_bearer,
            tasks: [administration, endpoint],
        }
    }

    fn local<T>(&self, body: T) -> Request<T> {
        let mut r = Request::new(body);
        if let Some(bearer) = &self.local_bearer {
            r.metadata_mut()
                .insert("authorization", bearer.parse().unwrap());
        }
        r
    }

    async fn grant(
        &mut self,
        subject: &str,
        role: pb::AccountRole,
    ) -> Result<pb::AccountMember, Status> {
        let r = self.local(pb::GrantAccountMemberRequest {
            subject: subject.into(),
            role: role as i32,
        });
        Ok(self.administration.grant(r).await?.into_inner())
    }

    async fn revoke(&mut self, subject: &str) -> Result<(), Status> {
        let r = self.local(pb::RevokeAccountMemberRequest {
            subject: subject.into(),
        });
        self.administration.revoke(r).await?;
        Ok(())
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[tokio::test]
async fn gateway_enforces_role_intersection_after_verified_assertion() {
    use pb::AccountRole::*;
    let mut fixture = Gateway::start().await;
    let mut rpc = pb::osdl_client::OsdlClient::new(fixture.external.clone());
    let operate = "opensdl:read opensdl:operate";
    let read = "opensdl:read";
    let request = || {
        authorized(
            pb::StatusRequest {},
            &fixture.key,
            member_claims(VIEWER, operate),
        )
    };
    let missing = rpc.status(request()).await.unwrap_err();
    assert_eq!(
        missing.metadata().get("opensdl-auth-error").unwrap(),
        "account_mismatch"
    );

    for (subject, role) in [(VIEWER, Viewer), (OPERATOR, Operator), (ADMIN, Admin)] {
        let granted = fixture.grant(subject, role).await.unwrap();
        assert_eq!(granted.subject, subject);
        assert_eq!(granted.role(), role);
    }
    let list = fixture.local(pb::ListAccountMembersRequest::default());
    let listed = fixture
        .administration
        .list_members(list)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.owner_subject, "account-owner");
    assert_eq!(listed.members.len(), 3);
    assert_eq!(
        fixture
            .grant("account-owner", Viewer)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        fixture.revoke("account-owner").await.unwrap_err().code(),
        Code::FailedPrecondition
    );

    for (subject, scope, can_operate, writable) in [
        (VIEWER, operate, false, false),
        (OPERATOR, read, false, false),
        (OPERATOR, operate, true, false),
        (ADMIN, read, false, false),
        (ADMIN, operate, true, true),
        ("account-owner", operate, true, true),
    ] {
        let identity = member_claims(subject, scope);
        let profile = rpc
            .status(authorized(
                pb::StatusRequest {},
                &fixture.key,
                identity.clone(),
            ))
            .await
            .unwrap()
            .into_inner()
            .laboratory
            .unwrap();
        assert_eq!(profile.writable, writable, "{subject} {scope}");
        let update = pb::UpdateLabProfileRequest {
            name: format!("Updated by {subject}"),
            revision: profile.revision,
            ..Default::default()
        };
        let result = rpc
            .update_lab_profile(authorized(update.clone(), &fixture.key, identity.clone()))
            .await;
        if writable {
            assert_eq!(result.unwrap().into_inner().name, update.name);
        } else {
            assert_eq!(result.unwrap_err().code(), Code::PermissionDenied);
        }
        let result = rpc
            .send_command(authorized(
                pb::SendCommandRequest {
                    device_id: "missing-device".into(),
                    action: "status".into(),
                    ..Default::default()
                },
                &fixture.key,
                identity,
            ))
            .await;
        // Only an admitted operator reaches the no-hardware upstream dispatch.
        let expected = if can_operate {
            Code::FailedPrecondition
        } else {
            Code::PermissionDenied
        };
        assert_eq!(result.unwrap_err().code(), expected, "{subject} {scope}");
    }

    for (field, value, expected) in [
        (
            "aud",
            json!("34444444-4444-4444-8444-444444444444"),
            "invalid_assertion",
        ),
        ("channel", json!("stable"), "invalid_assertion"),
        ("iss", json!("https://liyanlabs.com"), "invalid_assertion"),
        ("sub", json!(""), "invalid_assertion"),
        ("expired", json!(null), "authorization_expired"),
    ] {
        let mut c = member_claims(ADMIN, operate);
        if field == "expired" {
            let now = c["iat"].as_u64().unwrap();
            c["iat"] = json!(now - 300);
            c["exp"] = json!(now - 1);
        } else {
            c[field] = value;
        }
        let error = rpc
            .status(authorized(pb::StatusRequest {}, &fixture.key, c))
            .await
            .unwrap_err();
        assert_eq!(
            error.metadata().get("opensdl-auth-error").unwrap(),
            expected
        );
    }
    let forged = Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
    let error = rpc
        .status(authorized(
            pb::StatusRequest {},
            &forged,
            member_claims(ADMIN, operate),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.metadata().get("opensdl-auth-error").unwrap(),
        "invalid_assertion"
    );

    let mut remote_admin = AdminClient::new(fixture.external.clone());
    let error = remote_admin
        .list_members(authorized(
            pb::ListAccountMembersRequest::default(),
            &fixture.key,
            claims(),
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unimplemented);
}

#[tokio::test]
async fn live_changes_terminate_only_the_changed_subjects_stream_and_renewal() {
    let mut fixture = Gateway::start().await;
    fixture
        .grant(VIEWER, pb::AccountRole::Viewer)
        .await
        .unwrap();
    fixture
        .grant(OPERATOR, pb::AccountRole::Operator)
        .await
        .unwrap();

    let mut rpc = pb::osdl_client::OsdlClient::new(fixture.external.clone());
    let mut renew = pb::authentication_client::AuthenticationClient::new(fixture.external.clone());
    let read = "opensdl:read";
    let viewer = member_claims(VIEWER, read);
    let operator = member_claims(OPERATOR, read);
    let mut streams = Vec::new();
    // The signed device, generation and installation are deliberately identical.
    for identity in [&viewer, &operator] {
        streams.push(
            rpc.stream_events(authorized(
                pb::StreamEventsRequest::default(),
                &fixture.key,
                identity.clone(),
            ))
            .await
            .unwrap()
            .into_inner(),
        );
    }
    tokio::time::timeout(Duration::from_secs(3), fixture.revoke(VIEWER))
        .await
        .unwrap()
        .unwrap();
    let denied = tokio::time::timeout(Duration::from_secs(1), streams[0].message())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        denied.metadata().get("opensdl-auth-error").unwrap(),
        "account_mismatch"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(60), streams[1].message())
            .await
            .is_err()
    );

    let request = authorized(
        pb::AuthenticationRenewRequest {},
        &fixture.key,
        viewer.clone(),
    );
    assert_eq!(
        renew.renew(request).await.unwrap_err().code(),
        Code::Unauthenticated
    );
    let request = authorized(
        pb::AuthenticationRenewRequest {},
        &fixture.key,
        operator.clone(),
    );
    renew.renew(request).await.unwrap();

    let request = authorized(pb::StatusRequest {}, &fixture.key, viewer);
    assert_eq!(
        rpc.status(request).await.unwrap_err().code(),
        Code::Unauthenticated
    );

    tokio::time::timeout(
        Duration::from_secs(3),
        fixture.grant(OPERATOR, pb::AccountRole::Viewer),
    )
    .await
    .unwrap()
    .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), streams[1].message())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code(), Code::Unauthenticated);
    let request = authorized(pb::StatusRequest {}, &fixture.key, operator);
    rpc.status(request).await.unwrap();
}

#[test]
fn account_memberships_reload_bound_to_one_lab_and_a_single_writer() {
    let (_root, paths, auth, key) = bound_account();
    auth.members
        .as_ref()
        .unwrap()
        .grant(VIEWER.into(), super::members::Role::Viewer)
        .unwrap();
    auth.members
        .as_ref()
        .unwrap()
        .grant(OPERATOR.into(), super::members::Role::Operator)
        .unwrap();
    auth.members
        .as_ref()
        .unwrap()
        .revoke(VIEWER)
        .unwrap()
        .unwrap();
    assert!(AccountAuth::open(&paths.config_dir, "laboratory").is_err());
    drop(auth);

    let reloaded = AccountAuth::open(&paths.config_dir, "laboratory").unwrap();
    let operator = authorized(
        (),
        &key,
        member_claims(OPERATOR, "opensdl:read opensdl:operate"),
    );
    let viewer = authorized((), &key, member_claims(VIEWER, "opensdl:read"));
    let authority = Authorizer::Account(reloaded);
    assert!(authority.authorize(&operator, Permission::Operate).is_ok());
    assert_eq!(
        authority
            .authorize(&viewer, Permission::Read)
            .err()
            .unwrap()
            .code(),
        Code::Unauthenticated
    );

    let mut other = fixture().0;
    other.lab_id = "44444444-4444-4444-8444-444444444444".into();
    other.instance = "other-laboratory".into();
    other.save_new(&paths.config_dir).unwrap();
    let mut foreign_claim = member_claims(OPERATOR, "opensdl:read opensdl:operate");
    foreign_claim["aud"] = json!(other.lab_id);
    let request = authorized((), &key, foreign_claim);
    let authority =
        Authorizer::Account(AccountAuth::open(&paths.config_dir, &other.instance).unwrap());
    assert_eq!(
        authority
            .authorize(&request, Permission::Operate)
            .err()
            .unwrap()
            .code(),
        Code::Unauthenticated
    );
}
