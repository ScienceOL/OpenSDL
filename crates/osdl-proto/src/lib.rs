//! Generated gRPC types and stubs for OpenSDL.
//!
//! Wire definitions live in `proto/osdl.proto`. The build script invokes
//! `tonic-build` to produce the modules below; we re-export the package
//! flat at the crate root for convenience.

#![allow(clippy::all)]

pub mod v1 {
    tonic::include_proto!("osdl.v1");
}

pub use v1::*;

#[cfg(test)]
mod tests {
    #[test]
    fn enrollment_debug_never_contains_capabilities() {
        let request = super::PairRequest {
            capability: "capability-canary".into(),
            ..Default::default()
        };
        assert!(!format!("{request:?}").contains("capability-canary"));
    }
}

impl std::fmt::Debug for PairRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairRequest { redacted }")
    }
}
impl std::fmt::Debug for DescribeInvitationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DescribeInvitationRequest { redacted }")
    }
}
impl std::fmt::Debug for InviteResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InviteResponse { redacted }")
    }
}
