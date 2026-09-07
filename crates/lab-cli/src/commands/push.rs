use std::path::PathBuf;

use clap::Args;
use opensdl_assets::{AssetReference, RegistryClient};

use super::registry_auth::RegistryAuthArgs;

#[derive(Args)]
pub struct PushArgs {
    /// OCI image-layout directory produced by `lab pack`.
    pub layout: PathBuf,
    /// Tagged OCI destination, for example localhost:5001/lab/ur5e:1.0.0.
    pub reference: String,
    #[command(flatten)]
    pub auth: RegistryAuthArgs,
}

pub async fn run(args: PushArgs) -> anyhow::Result<()> {
    let reference = AssetReference::parse(&args.reference)?;
    let result = RegistryClient::for_reference(&reference)
        .push_layout(&args.layout, &reference, &args.auth.credentials())
        .await?;
    println!("{}", result.manifest_digest);
    Ok(())
}
