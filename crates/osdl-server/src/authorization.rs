//! One request boundary for listener authority and stream lifetime.
pub(crate) mod info;
pub(crate) mod lease;
pub(crate) mod stream;
use crate::account::AccountAuth;
use lease::Resource;

use tonic::{Request, Status};
#[derive(Clone, Default)]
pub enum Authorizer {
    #[default]
    Local,
    Account(AccountAuth),
    PublicKey(crate::public_key::PublicKeyAuth),
}
#[derive(Clone, Copy)]
pub(crate) enum Permission {
    Read,
    Operate,
    Shutdown,
}
impl Authorizer {
    pub(crate) fn authorize<T>(
        &self,
        request: &Request<T>,
        permission: Permission,
    ) -> Result<Option<Resource>, Status> {
        match self {
            Self::Local => Ok(None),
            Self::PublicKey(auth) => auth.authorize(request, permission).map(Some),
            Self::Account(auth) => {
                let identity = auth.verifier.authenticate(request)?;
                match permission {
                    Permission::Shutdown => {
                        return Err(Status::permission_denied("local_administrator_required"))
                    }
                    Permission::Operate if identity.scope != crate::account::Scope::Operate => {
                        return Err(Status::permission_denied("operate_required"))
                    }
                    _ => {}
                }
                auth.leases.acquire(&identity).resource().map(Some)
            }
        }
    }
}
