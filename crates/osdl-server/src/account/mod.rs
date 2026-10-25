//! Pinned laboratory binding and locally verified Runner identity assertions.
mod binding;
mod identity;
pub(crate) mod lease;
pub(crate) mod service;

pub use binding::{channel_for_origin, Binding};
pub(crate) use identity::AuthError;
pub use identity::{Identity, Principal, Scope, Verifier};
pub use service::AccountAuth;
pub(crate) use service::AuthenticationService;

#[cfg(test)]
mod tests;
