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
