//! Device-local pairing and credential storage. No engine or server dependency.
mod invitation;
mod store;
pub use invitation::{normalize_endpoint, Access, Channel, Invitation};
pub use store::ProfileStore;
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

mod client;
pub use client::{inspect_invitation, Laboratory, PendingProfile, ProfileMetadata, ProfileState};
