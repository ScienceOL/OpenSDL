use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use clap::Subcommand;
use opensdl_assets::{daemon, AssetCache};

#[derive(Debug, Subcommand)]
pub enum DaemonCmd {
    /// Start the local content-addressed asset store daemon.
    Start {
        /// Keep the daemon attached to this terminal.
        #[arg(long)]
        foreground: bool,
        #[arg(long)]
        json: bool,
    },
    /// Run the daemon in the foreground. Used by service managers and Start.
    #[command(hide = true)]
    Serve,
    /// Stop the local asset store daemon.
    Stop {
        #[arg(long)]
        json: bool,
    },
    /// Show daemon health and endpoint.
    Status {
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(command: DaemonCmd, root: Option<PathBuf>) -> anyhow::Result<()> {
    let cache = match root {
        Some(root) => AssetCache::open(root)?,
        None => AssetCache::discover()?,
    };
    match command {
        DaemonCmd::Serve => daemon::serve(cache).await?,
        DaemonCmd::Start { foreground, json } => {
            if let Some(client) = daemon::DaemonClient::discover(&cache).await? {
                let record = client.record();
                if json {
                    println!("{}", serde_json::to_string(record)?);
                } else {
                    println!(
                        "asset daemon already running (pid {}, {})",
                        record.pid, record.endpoint
                    );
                }
                return Ok(());
            }
            if foreground {
                daemon::serve(cache).await?;
                return Ok(());
            }
            let executable = std::env::current_exe().context("locate lab executable")?;
            let mut child = Command::new(executable);
            child
                .arg("--cache-dir")
                .arg(cache.root())
                .args(["daemon", "serve"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("start asset daemon")?;
            let client = daemon::DaemonClient::wait(&cache, Duration::from_secs(5)).await?;
            if json {
                println!("{}", serde_json::to_string(client.record())?);
            } else {
                println!(
                    "asset daemon started (pid {}, {})",
                    client.record().pid,
                    client.record().endpoint
                );
            }
        }
        DaemonCmd::Stop { json } => {
            let Some(client) = daemon::DaemonClient::discover(&cache).await? else {
                if json {
                    println!("{{\"running\":false}}");
                } else {
                    println!("asset daemon is not running");
                }
                return Ok(());
            };
            client.call("shutdown", serde_json::json!({})).await?;
            if json {
                println!("{{\"running\":false}}");
            } else {
                println!("asset daemon stopped");
            }
        }
        DaemonCmd::Status { json } => {
            let record = daemon::DaemonClient::discover(&cache)
                .await?
                .map(|client| client.record().clone());
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "running": record.is_some(),
                        "pid": record.as_ref().map(|record| record.pid),
                        "endpoint": record.as_ref().map(|record| &record.endpoint),
                    })
                );
            } else if let Some(record) = record {
                println!("running pid={} endpoint={}", record.pid, record.endpoint);
            } else {
                println!("stopped");
            }
        }
    }
    Ok(())
}
