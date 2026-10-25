//! Closed native-client failures. No remote text or private paths enter this value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientError {
    Transport,
    Trust,
    Invitation,
    Persistence,
    PairingRequired,
    CertificateExpired,
    ProfileExists,
    InvalidInput,
}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Transport => "service_unreachable",
            Self::Trust => "identity_changed",
            Self::Invitation => "invalid_invitation",
            Self::Persistence => "local_persistence_failed",
            Self::PairingRequired => "pairing_required",
            Self::CertificateExpired => "grant_expired",
            Self::ProfileExists => "profile_exists",
            Self::InvalidInput => "invalid_pairing_request",
        })
    }
}
impl std::error::Error for ClientError {}

impl ClientError {
    pub(crate) fn transport(error: tonic::transport::Error) -> Self {
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
        while let Some(current) = cause {
            if current.is::<rustls::Error>() {
                return Self::Trust;
            }
            // std::io::Error's source may skip the wrapped error itself.
            if let Some(io) = current.downcast_ref::<std::io::Error>() {
                if let Some(inner) = io.get_ref() {
                    cause = Some(inner);
                    continue;
                }
            }
            cause = current.source();
        }
        Self::Transport
    }
    pub(crate) fn enrollment(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::InvalidArgument
            | tonic::Code::Unauthenticated
            | tonic::Code::PermissionDenied
            | tonic::Code::NotFound
            | tonic::Code::AlreadyExists
            | tonic::Code::FailedPrecondition => Self::Invitation,
            _ => Self::Transport,
        }
    }
    pub(crate) fn discovery(status: tonic::Status) -> Self {
        match status.code() {
            tonic::Code::Unauthenticated | tonic::Code::PermissionDenied => Self::Trust,
            _ => Self::Transport,
        }
    }
}
