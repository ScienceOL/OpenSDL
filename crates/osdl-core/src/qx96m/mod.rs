//! Native QX96M equipment status reader.
//!
//! Protocol source: ScienceOL/Liyan bc4a6e94648838d3c991b8c8472eaad63bc62121,
//! plugins/qx96m/internal/instrument and provider/projection.go.

mod client;
mod station;
mod wire;

pub(crate) use station::{Adapter, Runtime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum Error {
    #[error("QX96M connection unavailable")]
    Unavailable,
    #[error("QX96M rejected the request")]
    Rejected,
    #[error("QX96M returned an invalid reply")]
    Protocol,
}

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Unavailable
    }
}
