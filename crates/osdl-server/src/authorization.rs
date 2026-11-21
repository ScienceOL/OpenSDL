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
    /// Filesystem- or capability-protected endpoint in explicit local mode.
    ProtectedLocal,
    /// The caller must terminate mutually authenticated control-plane TLS.
    ControlPlane,
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
        self.evaluate(request, permission).map(|(lease, _)| lease)
    }

    pub(crate) fn lab_profile_writable<T>(&self, request: &Request<T>) -> Result<bool, Status> {
        self.evaluate(request, Permission::Read)
            .map(|(_, writable)| writable)
    }

    fn evaluate<T>(
        &self,
        request: &Request<T>,
        permission: Permission,
    ) -> Result<(Option<Resource>, bool), Status> {
        match self {
            Self::Local => Ok((None, false)),
            Self::ProtectedLocal => Ok((None, true)),
            Self::ControlPlane => match permission {
                Permission::Shutdown => {
                    Err(Status::permission_denied("local_administrator_required"))
                }
                _ => Ok((None, false)),
            },
            Self::PublicKey(auth) => Ok((Some(auth.authorize(request, permission)?), false)),
            Self::Account(auth) => {
                let identity = auth.verifier.authenticate(request)?;
                let can_operate = identity.scope == crate::account::Scope::Operate;
                match permission {
                    Permission::Shutdown => {
                        return Err(Status::permission_denied("local_administrator_required"))
                    }
                    Permission::Operate if !can_operate => {
                        return Err(Status::permission_denied("operate_required"))
                    }
                    _ => {}
                }
                let lease = auth.leases.acquire(&identity).resource()?;
                Ok((Some(lease), can_operate))
            }
        }
    }
}
