use super::{Binding, Verifier};
use crate::authorization::lease::Leases;
#[derive(Clone, Debug)]
pub struct AccountAuth {
    pub(crate) verifier: Verifier,
    pub(crate) leases: Leases,
}
impl AccountAuth {
    pub fn new(binding: Binding) -> anyhow::Result<Self> {
        Ok(Self {
            verifier: Verifier::new(binding)?,
            leases: Leases::default(),
        })
    }
}
