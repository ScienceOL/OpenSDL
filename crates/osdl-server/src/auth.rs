//! Bearer-token auth interceptor for the TCP gRPC listener.
//!
//! Threat model: a routable lab-network endpoint where anyone who can
//! reach the port could otherwise drive lab hardware (`SendCommand`)
//! and shut the server down (`Shutdown`). A shared bearer token closes
//! the obvious hole. mTLS is the longer-term answer; this is the
//! minimum we ship today.
//!
//! UDS clients skip this check — filesystem perms (0600 in `serve()`)
//! are the auth there.
//!
//! Wire format: gRPC clients pass `authorization: Bearer <TOKEN>` in
//! request metadata. We compare in constant time so a server log
//! showing a single byte mismatch can't be used to leak the token.

use tonic::{metadata::MetadataValue, service::Interceptor, Request, Status};

/// Build a tonic interceptor that requires `authorization: Bearer
/// <expected>` on every RPC. Returns `unauthenticated` otherwise.
pub fn bearer_interceptor(expected: String) -> impl Interceptor + Clone {
    BearerCheck { expected }
}

#[derive(Clone)]
struct BearerCheck {
    expected: String,
}

impl Interceptor for BearerCheck {
    fn call(&mut self, req: Request<()>) -> Result<Request<()>, Status> {
        let header = req
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("missing authorization header"))?;
        if !verify_bearer(header, &self.expected) {
            return Err(Status::unauthenticated("invalid authorization token"));
        }
        Ok(req)
    }
}

fn verify_bearer(header: &MetadataValue<tonic::metadata::Ascii>, expected: &str) -> bool {
    let raw = match header.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let presented = match raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer ")) {
        Some(t) => t,
        None => return false,
    };
    constant_time_eq(presented.as_bytes(), expected.as_bytes())
}

/// Length-aware constant-time byte comparison. Returns false for
/// length-mismatched inputs but still walks one of the two buffers
/// fully so the timing reveals only the length, never per-byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        // Touch a so the optimizer doesn't shortcut on length-mismatch
        // alone, leaking nothing more than what `a.len() != b.len()`
        // already does.
        let mut sink: u8 = 0;
        for byte in a {
            sink ^= *byte;
        }
        std::hint::black_box(sink);
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_token_accepted() {
        let h = MetadataValue::try_from("Bearer hunter2").unwrap();
        assert!(verify_bearer(&h, "hunter2"));
    }

    #[test]
    fn wrong_token_rejected() {
        let h = MetadataValue::try_from("Bearer hunter3").unwrap();
        assert!(!verify_bearer(&h, "hunter2"));
    }

    #[test]
    fn missing_bearer_prefix_rejected() {
        let h = MetadataValue::try_from("hunter2").unwrap();
        assert!(!verify_bearer(&h, "hunter2"));
    }

    #[test]
    fn lowercase_bearer_accepted() {
        let h = MetadataValue::try_from("bearer hunter2").unwrap();
        assert!(verify_bearer(&h, "hunter2"));
    }

    #[test]
    fn length_mismatch_rejected() {
        let h = MetadataValue::try_from("Bearer hunter").unwrap();
        assert!(!verify_bearer(&h, "hunter2"));
    }

    #[test]
    fn ct_eq_basics() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }
}
