use super::registry_auth::RegistryAuthArgs;
use anyhow::Context;
use clap::Subcommand;
use opensdl_assets::{AssetCache, AssetError, AssetReference, RegistryClient};
use osdl_server::managed::ManagedHost;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum InstanceCmd {
    /// Create a stopped instance from a runnable package.
    Create {
        reference: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        offline: bool,
        #[command(flatten)]
        auth: RegistryAuthArgs,
    },
    /// Create and start a runnable local instance.
    Run {
        reference: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        offline: bool,
        #[command(flatten)]
        auth: RegistryAuthArgs,
    },
    Start {
        name: String,
        #[arg(long)]
        json: bool,
    },
    Stop {
        name: String,
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    Inspect {
        name: String,
        #[arg(long)]
        json: bool,
    },
    Remove {
        name: String,
    },
    Logs {
        name: String,
        #[arg(long, default_value_t = 100)]
        tail: usize,
    },
    Devices {
        name: String,
        #[arg(long)]
        json: bool,
    },
    Send {
        name: String,
        device: String,
        action: String,
        #[arg(long, default_value = "{}")]
        params: String,
        #[arg(long)]
        json: bool,
    },
}
fn output(value: impl serde::Serialize, json: bool) -> anyhow::Result<()> {
    println!(
        "{}",
        if json {
            serde_json::to_string(&value)?
        } else {
            serde_json::to_string_pretty(&value)?
        }
    );
    Ok(())
}
pub async fn run(command: InstanceCmd, root: Option<PathBuf>) -> anyhow::Result<()> {
    let cache = match root {
        Some(root) => AssetCache::open(root)?,
        None => AssetCache::discover()?,
    };
    let host = ManagedHost::open(cache.clone())?;
    let executable = std::env::current_exe().context("locate lab executable")?;
    let start = matches!(command, InstanceCmd::Run { .. });
    match command {
        InstanceCmd::Create {
            reference,
            name,
            json,
            offline,
            auth,
        }
        | InstanceCmd::Run {
            reference,
            name,
            json,
            offline,
            auth,
        } => {
            match cache.resolve(&reference) {
                Ok(_) => {}
                Err(AssetError::NotFound(_)) => {
                    let reference = AssetReference::parse(&reference)?;
                    RegistryClient::for_reference(&reference)
                        .pull(&reference, &cache, &auth.credentials(), offline)
                        .await?;
                }
                Err(error) => return Err(error.into()),
            }
            let created = host.create(&reference, &name)?;
            output(
                if start {
                    host.start(&name, &executable).await?
                } else {
                    created
                },
                json,
            )?;
        }
        InstanceCmd::Start { name, json } => output(host.start(&name, &executable).await?, json)?,
        InstanceCmd::Stop { name, json } => output(host.stop(&name).await?, json)?,
        InstanceCmd::List { all, json } => output(host.list(all)?, json)?,
        InstanceCmd::Inspect { name, json } => output(host.inspect(&name)?, json)?,
        InstanceCmd::Remove { name } => host.remove(&name)?,
        InstanceCmd::Logs { name, tail } => println!("{}", host.logs(&name, tail)?),
        InstanceCmd::Devices { name, json } => output(host.devices(&name).await?, json)?,
        InstanceCmd::Send {
            name,
            device,
            action,
            params,
            json,
        } => output(
            host.send(&name, &device, &action, serde_json::from_str(&params)?)
                .await?,
            json,
        )?,
    }
    Ok(())
}
