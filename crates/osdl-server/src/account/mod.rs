//! Pinned laboratory binding and locally verified Runner identity assertions.
mod binding;
mod identity;
pub(crate) mod service;

pub use binding::{channel_for_origin, Binding};
pub(crate) use identity::AuthError;
pub use identity::{Identity, Principal, Scope, Verifier};
pub use service::AccountAuth;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stream_tests;
