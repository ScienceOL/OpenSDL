//! Device-local pairing and credential storage. No engine or server dependency.
mod invitation;
mod store;
pub use invitation::{normalize_endpoint, Access, Channel, Invitation};
pub use store::ProfileStore;
/// One instance identity is used by account bindings, private discovery and lockfiles.
pub fn valid_instance_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
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
mod client_error;
pub use client::{inspect_invitation, Laboratory, PendingProfile, ProfileMetadata, ProfileState};
pub use client_error::ClientError;

#[cfg(test)]
mod instance_tests {
    #[test]
    fn instance_names_share_one_filesystem_safe_alphabet() {
        for valid in ["default", "thinkpad-lab", "LAB_9"] {
            assert!(super::valid_instance_name(valid));
        }
        for invalid in ["", "lab.d", "../lab", "lab/name", "lab name", ".", ".."] {
            assert!(!super::valid_instance_name(invalid));
        }
        assert!(!super::valid_instance_name(&"a".repeat(129)));
        assert!(super::valid_instance_name(&"a".repeat(128)));
    }
}
