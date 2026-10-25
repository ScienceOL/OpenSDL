fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use the vendored `protoc` so contributors don't have to install it
    // separately; this matters for CI and the various lab machines.
    if std::env::var_os("PROTOC").is_none() {
        let path = protoc_bin_vendored::protoc_bin_path()
            .map_err(|e| format!("vendored protoc unavailable: {e}"))?;
        std::env::set_var("PROTOC", path);
    }

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .skip_debug([
            ".osdl.v1.PairRequest",
            ".osdl.v1.DescribeInvitationRequest",
            ".osdl.v1.InviteResponse",
        ])
        .compile_protos(&["proto/osdl.proto"], &["proto"])?;
    Ok(())
}
