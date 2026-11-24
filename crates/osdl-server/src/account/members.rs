//! Durable membership rows and an in-memory index for authorization.
use super::{binding::valid_subject, Binding};
use crate::authorization::lease::{Lease, Leases};
use anyhow::{ensure, Context, Result};
use osdl_auth::ProfileStore;
use osdl_proto::v1 as pb;
use rusqlite::{params, Connection};
use std::{
    collections::BTreeMap,
    fs::File,
    ops::Bound::{Excluded, Unbounded},
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Viewer,
    Operator,
    Admin,
}

impl Role {
    pub(super) fn from_wire(value: i32) -> Option<Self> {
        match pb::AccountRole::try_from(value).ok()? {
            pb::AccountRole::Viewer => Some(Self::Viewer),
            pb::AccountRole::Operator => Some(Self::Operator),
            pb::AccountRole::Admin => Some(Self::Admin),
            pb::AccountRole::Unspecified => None,
        }
    }

    fn member(self, subject: String) -> pb::AccountMember {
        let role = match self {
            Self::Viewer => pb::AccountRole::Viewer,
            Self::Operator => pb::AccountRole::Operator,
            Self::Admin => pb::AccountRole::Admin,
        };
        pb::AccountMember {
            subject,
            role: role as i32,
        }
    }

    pub(super) fn allows_operations(self) -> bool {
        matches!(self, Self::Operator | Self::Admin)
    }

    pub(super) fn allows_profile_edit(self) -> bool {
        self == Self::Admin
    }
}

#[derive(Debug)]
struct Member {
    role: Role,
    leases: Leases,
}

#[derive(Debug)]
pub(super) struct Members {
    db: Mutex<Connection>,
    index: RwLock<BTreeMap<String, Member>>,
    _writer: File,
}

impl Members {
    pub(super) fn open(config: &Path, binding: &Binding) -> Result<Self> {
        let path = Binding::path(config, &binding.instance)?;
        // Include the exact issuer, lab and bound owner in the database scope.
        // A replacement binding cannot inherit memberships automatically.
        use sha2::{Digest, Sha256};
        let binding_scope =
            serde_json::to_vec(&(&binding.issuer, &binding.lab_id, &binding.account_id))?;
        let root = path
            .parent()
            .expect("binding parent")
            .canonicalize()?
            .join(format!("members-{:x}", Sha256::digest(binding_scope)));
        let store = ProfileStore::open(root)?;
        let writer = store.lock("writer")?;
        let db = Connection::open(store.root().join("members.sqlite"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS members(
                subject TEXT PRIMARY KEY NOT NULL,
                role INTEGER NOT NULL CHECK(role IN (1, 2, 3))
             ) WITHOUT ROWID;",
        )?;
        let mut index = BTreeMap::new();
        {
            let mut statement = db.prepare("SELECT subject, role FROM members")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let subject: String = row.get(0)?;
                ensure!(
                    valid_subject(&subject) && subject != binding.account_id,
                    "invalid stored account subject"
                );
                let role = Role::from_wire(row.get(1)?).context("invalid stored account role")?;
                index.insert(
                    subject,
                    Member {
                        role,
                        leases: Leases::default(),
                    },
                );
            }
        }
        Ok(Self {
            _writer: writer,
            db: Mutex::new(db),
            index: RwLock::new(index),
        })
    }

    pub(super) fn access<T>(
        &self,
        subject: &str,
        use_member: impl FnOnce(Role, &Leases) -> T,
    ) -> Option<T> {
        // Keep the read guard through lease acquisition or renewal. A writer
        // can then invalidate all leases for the old role before publishing.
        let index = self.index.read().unwrap();
        index
            .get(subject)
            .map(|member| use_member(member.role, &member.leases))
    }

    pub(super) fn list(&self, owner: &str, after: &str) -> pb::ListAccountMembersResponse {
        let index = self.index.read().unwrap();
        let mut rows = index.range::<str, _>((Excluded(after), Unbounded));
        let members: Vec<_> = rows
            .by_ref()
            .take(PAGE_SIZE)
            .map(|(subject, member)| member.role.member(subject.clone()))
            .collect();
        let next_subject = if rows.next().is_some() {
            members.last().expect("full page").subject.clone()
        } else {
            String::new()
        };
        pb::ListAccountMembersResponse {
            owner_subject: owner.into(),
            members,
            next_subject,
        }
    }

    pub(super) fn grant(
        &self,
        subject: String,
        role: Role,
    ) -> Result<(pb::AccountMember, Vec<Arc<Lease>>)> {
        let db = self.db.lock().unwrap();
        let account = role.member(subject.clone());
        let affected = db.execute(
            "INSERT INTO members(subject, role) VALUES(?1, ?2)
             ON CONFLICT(subject) DO UPDATE SET role=excluded.role WHERE role<>excluded.role",
            params![subject, account.role],
        )?;
        let leases = if affected == 0 {
            Vec::new()
        } else {
            self.replace_member(subject, Some(role))
        };
        Ok((account, leases))
    }

    pub(super) fn revoke(&self, subject: &str) -> Result<Option<Vec<Arc<Lease>>>> {
        let db = self.db.lock().unwrap();
        let removed = db.execute("DELETE FROM members WHERE subject=?1", [subject])?;
        Ok((removed != 0).then(|| self.replace_member(subject.to_owned(), None)))
    }

    fn replace_member(&self, subject: String, role: Option<Role>) -> Vec<Arc<Lease>> {
        // DB commits finish before this short critical section. The database
        // guard serializes publishing without holding up the read path on IO.
        let mut index = self.index.write().unwrap();
        let previous = match role {
            Some(role) => index.insert(
                subject,
                Member {
                    role,
                    leases: Leases::default(),
                },
            ),
            None => index.remove(&subject),
        };
        previous
            .map(|member| member.leases.revoke_all())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod measurements {
    use crate::account::{
        rbac_tests::bound_account,
        tests::{claims, token},
        AccountAuth,
    };
    use crate::authorization::Permission;
    use prost::Message;
    use serde_json::json;
    use std::{thread, time::Instant};
    use tonic::{metadata::MetadataValue, Request};

    #[test]
    fn measure_account_admission_with_short_and_long_membership_lists() {
        const CONCURRENT_CLIENTS: usize = 8;
        const REQUESTS_PER_CLIENT: usize = 250;
        for member_count in [100, 10_000] {
            let (_root, paths, auth, key) = bound_account();
            let store = auth.members.as_ref().unwrap();
            let write_started = Instant::now();
            store
                .db
                .lock()
                .unwrap()
                .execute(
                    "WITH RECURSIVE entries(i) AS (
                       VALUES(1) UNION ALL SELECT i+1 FROM entries WHERE i<?1
                     ) INSERT INTO members SELECT printf('test-subject-%05d', i), 2 FROM entries",
                    [member_count as i64],
                )
                .unwrap();
            let batch_write = write_started.elapsed();
            drop(auth);
            let load_started = Instant::now();
            let auth = AccountAuth::open(&paths.config_dir, "laboratory").unwrap();
            let load = load_started.elapsed();
            let credentials: Vec<_> = (0..CONCURRENT_CLIENTS)
                .map(|i| {
                    let mut c = claims();
                    c["sub"] = json!(format!("test-subject-{:05}", member_count - i));
                    MetadataValue::try_from(format!("Bearer {}", token(&key, c))).unwrap()
                })
                .collect();
            let auth_bytes = credentials[0].as_bytes().len();
            let first_page = auth.members.as_ref().unwrap().list("account-owner", "");
            let page_bytes = first_page.encoded_len();
            assert_eq!(first_page.members.len(), 100);
            if member_count > 100 {
                let next_page = auth
                    .members
                    .as_ref()
                    .unwrap()
                    .list("account-owner", &first_page.next_subject);
                assert_eq!(next_page.members.len(), 100);
                assert_eq!(next_page.members[0].subject, "test-subject-00101");
            } else {
                assert!(first_page.next_subject.is_empty());
            }
            let start = Instant::now();
            thread::scope(|scope| {
                for credential in credentials {
                    let auth = &auth;
                    scope.spawn(move || {
                        for _ in 0..REQUESTS_PER_CLIENT {
                            let mut request = Request::new(());
                            request
                                .metadata_mut()
                                .insert("authorization", credential.clone());
                            assert!(auth.authorize(&request, Permission::Operate).is_ok());
                        }
                    });
                }
            });
            println!(
                "PERF members={member_count} batch_write_us={} reload_us={} concurrent_requests={} elapsed_us={} auth_metadata_bytes={} first_list_page_bytes={} admission_sql_queries=0",
                batch_write.as_micros(),
                load.as_micros(),
                CONCURRENT_CLIENTS * REQUESTS_PER_CLIENT,
                start.elapsed().as_micros(),
                auth_bytes,
                page_bytes,
            );
        }
    }
}
