//! Laboratory-private CA, one writer, and constant-time-in-history grant admission.
mod identity;
mod rpc;
use crate::authorization::lease::Lease;
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use osdl_auth::{now, Access, Channel, Invitation, ProfileStore};
use osdl_proto::v1 as pb;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Arc, Mutex, RwLock},
};
struct Authority {
    grant: pb::Grant,
    lease: Arc<Lease>,
}
#[derive(Default)]
struct GrantIndex {
    entries: HashMap<Vec<u8>, Arc<Authority>>,
    deadlines: BTreeMap<(u64, String), Vec<u8>>,
}
impl GrantIndex {
    fn insert(&mut self, leaf: Vec<u8>, authority: Arc<Authority>) {
        self.remove(&leaf);
        while self
            .deadlines
            .first_key_value()
            .is_some_and(|(deadline, leaf)| {
                deadline.0 <= now()
                    && self
                        .entries
                        .get(leaf)
                        .is_none_or(|authority| authority.lease.is_drained())
            })
        {
            let (_, expired) = self.deadlines.pop_first().unwrap();
            self.entries.remove(&expired);
        }
        self.deadlines.insert(
            (
                authority.grant.certificate_expires_at as u64,
                authority.grant.id.clone(),
            ),
            leaf.clone(),
        );
        self.entries.insert(leaf, authority);
    }
    fn get(&self, leaf: &[u8]) -> Option<&Arc<Authority>> {
        self.entries.get(leaf)
    }
    fn remove(&mut self, leaf: &[u8]) -> Option<Arc<Authority>> {
        let authority = self.entries.remove(leaf)?;
        self.deadlines.remove(&(
            authority.grant.certificate_expires_at as u64,
            authority.grant.id.clone(),
        ));
        Some(authority)
    }
}
struct State {
    _writer: std::fs::File,
    identity: identity::Identity,
    endpoint: String,
    db: Mutex<Connection>,
    grants: Arc<RwLock<GrantIndex>>,
}
#[derive(Clone)]
pub struct PublicKeyAuth(Arc<State>);
impl PublicKeyAuth {
    pub fn open(path: &Path, instance: &str, channel: Channel, endpoint: &str) -> Result<Self> {
        ensure!(osdl_auth::valid_instance_name(instance), "invalid_instance");
        ensure!(
            osdl_auth::normalize_endpoint(endpoint)? == endpoint,
            "invalid_endpoint"
        );
        let store = ProfileStore::open(path.to_path_buf())?;
        let established = store.contains("writer")?;
        let writer = store.lock("writer")?;
        ensure!(
            !established || store.contains("identity")?,
            "laboratory_identity_missing"
        );
        if store.contains("identity")? {
            ensure!(
                path.join("grants.sqlite").is_file(),
                "laboratory_grants_missing"
            );
        }
        let identity = identity::Identity::load(&store, instance, channel)?;
        let db = Connection::open(path.join("grants.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS grants(id TEXT PRIMARY KEY,label TEXT NOT NULL,access INTEGER NOT NULL,expires INTEGER,cert_expires INTEGER NOT NULL,revoked INTEGER NOT NULL,leaf BLOB UNIQUE NOT NULL,cert TEXT NOT NULL); CREATE TABLE IF NOT EXISTS invitations(id TEXT PRIMARY KEY,hash BLOB NOT NULL,expires INTEGER NOT NULL,access INTEGER NOT NULL,grant_expires INTEGER,csr BLOB,grant_id TEXT);")?;
        let mut grants = GrantIndex::default();
        {
            let mut statement=db.prepare("SELECT id,label,access,expires,cert_expires,revoked,leaf FROM grants WHERE revoked=0 AND cert_expires>? AND (expires IS NULL OR expires>?)")?;
            for row in statement.query_map(params![now() as i64, now() as i64], |r| {
                Ok((read_grant(r)?, r.get::<_, Vec<u8>>(6)?))
            })? {
                let (grant, leaf) = row?;
                let lease = Lease::grant(grant.certificate_expires_at as u64);
                grants.insert(leaf, Arc::new(Authority { grant, lease }));
            }
        }
        Ok(Self(Arc::new(State {
            _writer: writer,
            identity,
            endpoint: endpoint.into(),
            db: Mutex::new(db),
            grants: Arc::new(RwLock::new(grants)),
        })))
    }
    pub fn lab_id(&self) -> &str {
        &self.0.identity.lab_id
    }
    pub fn instance(&self) -> &str {
        &self.0.identity.instance
    }
    pub fn channel(&self) -> &str {
        self.0.identity.channel.as_str()
    }
    pub fn tls(&self) -> tonic::transport::ServerTlsConfig {
        self.0.identity.tls()
    }
    pub fn invite(&self, access: Access, expiry: Option<u64>) -> Result<Invitation> {
        ensure!(
            expiry.is_none_or(|v| v > now() && v <= now() + 365 * 20 * 86400),
            "invalid_grant_expiry"
        );
        let db = self.0.db.lock().unwrap_or_else(|e| e.into_inner());
        db.execute("DELETE FROM invitations WHERE expires<=?", [now() as i64])?;
        let count: u32 = db.query_row("SELECT COUNT(*) FROM invitations", [], |r| r.get(0))?;
        ensure!(count < 256, "invitation_limit");
        let mut secret = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut secret)
            .map_err(|_| anyhow::anyhow!("entropy_unavailable"))?;
        let inv = Invitation {
            lab_id: self.lab_id().into(),
            instance: self.instance().into(),
            channel: self.0.identity.channel,
            endpoint: self.0.endpoint.clone(),
            ca_pem: self.0.identity.ca.clone(),
            invitation_id: uuid::Uuid::new_v4().to_string(),
            capability: B64.encode(secret),
            expires_at: now() + 600,
            access,
            grant_expires_at: expiry,
        };
        db.execute(
            "INSERT INTO invitations(id,hash,expires,access,grant_expires) VALUES(?,?,?,?,?)",
            params![
                inv.invitation_id,
                Sha256::digest(secret).as_slice(),
                inv.expires_at as i64,
                access_value(access),
                expiry.map(|v| v as i64)
            ],
        )?;
        Ok(inv)
    }
    pub fn describe(&self, id: &str, capability: &str) -> Result<pb::DescribeInvitationResponse> {
        let db = self.0.db.lock().unwrap_or_else(|e| e.into_inner());
        let invite = check_invitation(&db, id, capability)?;
        Ok(pb::DescribeInvitationResponse {
            lab_id: self.lab_id().into(),
            instance: self.instance().into(),
            channel: self.channel().into(),
            access: invite.access,
            grant_expires_at: invite.grant_expiry.map(|v| v as i64),
            invitation_expires_at: invite.expiry as i64,
        })
    }
    pub fn pair(&self, request: &pb::PairRequest) -> Result<pb::PairResponse> {
        ensure!(
            !request.label.is_empty()
                && request.label.len() <= 128
                && request.csr_der.len() <= 4096,
            "invalid_pair_request"
        );
        let csr_hash = Sha256::digest(&request.csr_der).to_vec();
        let mut db = self.0.db.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.transaction()?;
        let invitation = check_invitation(&tx, &request.invitation_id, &request.capability)?;
        if let Some(grant_id) = invitation.grant_id {
            ensure!(
                invitation.csr.as_ref() == Some(&csr_hash),
                "invitation_consumed"
            );
            let (grant, cert) = tx.query_row(
                "SELECT id,label,access,expires,cert_expires,revoked,cert FROM grants WHERE id=?",
                [grant_id],
                |r| Ok((read_grant(r)?, r.get(6)?)),
            )?;
            ensure!(!grant.revoked, "grant_revoked");
            return Ok(self.response(grant, cert));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let expiry = invitation.grant_expiry.unwrap_or(now() + 365 * 20 * 86400);
        ensure!(expiry > now(), "grant_expired");
        let (cert, der) = self.0.identity.issue(&request.csr_der, &id, expiry)?;
        let leaf = Sha256::digest(&der).to_vec();
        let grant = pb::Grant {
            id: id.clone(),
            label: request.label.clone(),
            access: invitation.access,
            expires_at: invitation.grant_expiry.map(|v| v as i64),
            certificate_expires_at: expiry as i64,
            revoked: false,
        };
        tx.execute(
            "INSERT INTO grants VALUES(?,?,?,?,?,0,?,?)",
            params![
                id,
                request.label,
                invitation.access,
                invitation.grant_expiry.map(|v| v as i64),
                expiry as i64,
                leaf,
                cert
            ],
        )?;
        tx.execute(
            "UPDATE invitations SET csr=?,grant_id=? WHERE id=?",
            params![csr_hash, id, request.invitation_id],
        )?;
        tx.commit()?;
        self.0
            .grants
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                leaf,
                Arc::new(Authority {
                    grant: grant.clone(),
                    lease: Lease::grant(expiry),
                }),
            );
        Ok(self.response(grant, cert))
    }
    fn response(&self, grant: pb::Grant, certificate_pem: String) -> pb::PairResponse {
        pb::PairResponse {
            grant: Some(grant),
            certificate_pem,
            lab_id: self.lab_id().into(),
            channel: self.channel().into(),
        }
    }
    pub fn list(&self, after: &str, limit: u32) -> Result<pb::ListGrantsResponse> {
        ensure!(after.len() <= 36, "invalid_cursor");
        let limit = if limit == 0 { 100 } else { limit.min(100) };
        let db = self.0.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut s=db.prepare("SELECT id,label,access,expires,cert_expires,revoked FROM grants WHERE id>? ORDER BY id LIMIT ?")?;
        let grants = s
            .query_map(params![after, limit], read_grant)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_id = if grants.len() == limit as usize {
            grants.last().unwrap().id.clone()
        } else {
            String::new()
        };
        Ok(pb::ListGrantsResponse { grants, next_id })
    }
    pub async fn revoke(&self, id: &str) -> Result<()> {
        let auth = self.clone();
        let id = id.to_owned();
        let lease = tokio::task::spawn_blocking(move || auth.revoke_authority(&id)).await??;
        if let Some(lease) = lease {
            lease.drained().await;
        }
        Ok(())
    }
    fn revoke_authority(&self, id: &str) -> Result<Option<Arc<Lease>>> {
        let lease = {
            let db = self.0.db.lock().unwrap_or_else(|e| e.into_inner());
            let leaf: Vec<u8> =
                db.query_row("SELECT leaf FROM grants WHERE id=?", [id], |r| r.get(0))?;
            db.execute("UPDATE grants SET revoked=1 WHERE id=?", [id])?;
            let lease = self
                .0
                .grants
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(&leaf)
                .map(|a| a.lease.clone());
            if let Some(lease) = &lease {
                if lease.revoke() {
                    // Cleanup belongs to the authority even if the administering RPC is cancelled.
                    // Keep the lease indexed until drained so concurrent revokes await the same owners.
                    let grants = self.0.grants.clone();
                    let lease = lease.clone();
                    tokio::spawn(async move {
                        lease.drained().await;
                        grants
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&leaf);
                    });
                }
            }
            lease
        };
        Ok(lease)
    }
    pub(crate) fn authorize<T>(
        &self,
        request: &tonic::Request<T>,
        permission: crate::authorization::Permission,
    ) -> Result<crate::authorization::lease::Resource, tonic::Status> {
        let certs = request
            .peer_certs()
            .ok_or_else(|| status("client_certificate_required"))?;
        let leaf = certs
            .first()
            .ok_or_else(|| status("client_certificate_required"))?;
        self.authorize_leaf(leaf.as_ref(), permission)
    }
    fn authorize_leaf(
        &self,
        leaf: &[u8],
        permission: crate::authorization::Permission,
    ) -> Result<crate::authorization::lease::Resource, tonic::Status> {
        let hash = Sha256::digest(leaf);
        let index = self.0.grants.read().unwrap_or_else(|e| e.into_inner());
        let authority = index
            .get(hash.as_slice())
            .ok_or_else(|| status("grant_inactive"))?;
        if authority.grant.certificate_expires_at <= now() as i64 {
            return Err(status("grant_expired"));
        }
        match permission {
            crate::authorization::Permission::Shutdown => {
                return Err(tonic::Status::permission_denied(
                    "local_administrator_required",
                ))
            }
            crate::authorization::Permission::Operate if authority.grant.access != 2 => {
                return Err(tonic::Status::permission_denied("operate_required"))
            }
            _ => {}
        }
        authority.lease.resource()
    }
}
fn read_grant(r: &rusqlite::Row<'_>) -> rusqlite::Result<pb::Grant> {
    Ok(pb::Grant {
        id: r.get(0)?,
        label: r.get(1)?,
        access: r.get(2)?,
        expires_at: r.get(3)?,
        certificate_expires_at: r.get(4)?,
        revoked: r.get(5)?,
    })
}
fn access_value(access: Access) -> i32 {
    match access {
        Access::Read => 1,
        Access::Operate => 2,
    }
}
struct InviteRecord {
    expiry: u64,
    access: i32,
    grant_expiry: Option<u64>,
    csr: Option<Vec<u8>>,
    grant_id: Option<String>,
}
fn check_invitation(db: &Connection, id: &str, capability: &str) -> Result<InviteRecord> {
    ensure!(
        id.len() == 36 && capability.len() == 43,
        "invalid_invitation"
    );
    let secret = B64
        .decode(capability)
        .map_err(|_| anyhow::anyhow!("invalid_invitation"))?;
    ensure!(secret.len() == 32, "invalid_invitation");
    let result = db
        .query_row(
            "SELECT hash,expires,access,grant_expires,csr,grant_id FROM invitations WHERE id=?",
            [id],
            |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    InviteRecord {
                        expiry: r.get::<_, i64>(1)? as u64,
                        access: r.get(2)?,
                        grant_expiry: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                        csr: r.get(4)?,
                        grant_id: r.get(5)?,
                    },
                ))
            },
        )
        .optional()?;
    let (hash, record) = result.ok_or_else(|| anyhow::anyhow!("invalid_invitation"))?;
    ensure!(
        hash == Sha256::digest(secret).as_slice(),
        "invalid_invitation"
    );
    ensure!(record.expiry > now(), "invitation_expired");
    Ok(record)
}

#[cfg(test)]
mod tests;

pub(crate) fn status(code: &'static str) -> tonic::Status {
    let mut status = tonic::Status::unauthenticated(code);
    status.metadata_mut().insert(
        "opensdl-auth-error",
        tonic::metadata::MetadataValue::from_static(code),
    );
    status
}
