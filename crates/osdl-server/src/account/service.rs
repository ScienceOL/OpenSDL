use super::{
    members::{Members, Role},
    AuthError, Binding, Identity, Scope, Verifier,
};
use crate::authorization::{
    lease::{Leases, Resource},
    Permission,
};
use std::{path::Path, sync::Arc};
use tonic::{Request, Status};

#[derive(Clone, Debug)]
pub struct AccountAuth {
    pub(crate) verifier: Arc<Verifier>,
    pub(crate) leases: Leases,
    pub(super) members: Option<Arc<Members>>,
}

impl AccountAuth {
    pub fn new(binding: Binding) -> anyhow::Result<Self> {
        Ok(Self {
            verifier: Arc::new(Verifier::new(binding)?),
            leases: Leases::default(),
            members: None,
        })
    }

    pub fn open(config: &Path, instance: &str) -> anyhow::Result<Self> {
        let binding = Binding::load(config, instance)?;
        let members = Arc::new(Members::open(config, &binding)?);
        let mut auth = Self::new(binding)?;
        auth.members = Some(members);
        Ok(auth)
    }

    fn with_member<T>(
        &self,
        identity: &Identity,
        operation: impl FnOnce(Role, &Leases) -> T,
    ) -> Result<T, Status> {
        if identity.subject == self.verifier.binding().account_id {
            return Ok(operation(Role::Admin, &self.leases));
        }
        self.members
            .as_ref()
            .and_then(|members| members.access(&identity.subject, operation))
            .ok_or_else(|| AuthError::AccountMismatch.status())
    }

    pub(crate) fn authorize<T>(
        &self,
        request: &Request<T>,
        permission: Permission,
    ) -> Result<(Option<Resource>, bool), Status> {
        let identity = self.verifier.authenticate(request)?;
        self.with_member(&identity, |role, leases| {
            let operate_scope = identity.scope == Scope::Operate;
            let can_operate = operate_scope && role.allows_operations();
            let can_edit_profile = operate_scope && role.allows_profile_edit();
            match permission {
                Permission::Shutdown => {
                    return Err(Status::permission_denied("local_administrator_required"))
                }
                Permission::Operate if !can_operate => {
                    return Err(Status::permission_denied("operate_required"))
                }
                _ => {}
            }
            let resource = leases.acquire(&identity).resource()?;
            Ok((Some(resource), can_edit_profile))
        })?
    }

    pub(crate) fn renew<T>(&self, request: &Request<T>) -> Result<u64, Status> {
        let identity = self.verifier.authenticate(request)?;
        self.with_member(&identity, |_, leases| leases.renew(&identity))?;
        Ok(identity.expires_at)
    }
}
