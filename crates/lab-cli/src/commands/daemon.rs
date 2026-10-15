use anyhow::{bail, Context};
use clap::Subcommand;
use opensdl_assets::{
    daemon::{self, DaemonClient, Operation},
    AssetCache,
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

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
    /// Run the daemon in the foreground for a service manager.
    #[command(hide = true)]
    Serve,
    /// Drain accepted operations and stop the asset daemon.
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
        DaemonCmd::Start {
            foreground: true, ..
        } => daemon::serve(cache).await?,
        DaemonCmd::Start {
            foreground: false,
            json,
        } => {
            let client = match DaemonClient::discover(&cache).await? {
                Some(client) => client,
                None => start(&cache).await?,
            };
            let record = client.record();
            if json {
                println!("{}", serde_json::to_string(&record)?);
            } else {
                println!(
                    "asset daemon running (pid {}, {})",
                    record.pid, record.endpoint
                );
            }
        }
        DaemonCmd::Stop { json } => {
            if let Some(client) = DaemonClient::discover(&cache).await? {
                client.call(Operation::Shutdown).await?;
            }
            if json {
                println!("{{\"running\":false}}");
            } else {
                println!("asset daemon stopped");
            }
        }
        DaemonCmd::Status { json } => {
            let record = DaemonClient::discover(&cache)
                .await?
                .map(|client| client.record());
            if json {
                println!(
                    "{}",
                    serde_json::json!({"running": record.is_some(), "pid": record.as_ref().map(|r| r.pid), "endpoint": record.as_ref().map(|r| &r.endpoint)})
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

async fn start(cache: &AssetCache) -> anyhow::Result<DaemonClient> {
    let log_path = cache.root().join("daemon.log");
    if let Ok(metadata) = log_path.symlink_metadata() {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("daemon.log must be a regular file");
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options.open(&log_path).context("open daemon.log")?;
    let mut command = Command::new(std::env::current_exe().context("locate lab executable")?);
    command
        .arg("--cache-dir")
        .arg(cache.root())
        .args(["daemon", "serve"])
        .current_dir(cache.root())
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    for variable in [
        "LAB_REGISTRY_USERNAME",
        "LAB_REGISTRY_PASSWORD",
        "LAB_REGISTRY_TOKEN",
        "OSDL_AUTH_TOKEN",
    ] {
        command.env_remove(variable);
    }
    let mut child = osdl_server::background::spawn(&mut command).context("spawn asset daemon")?;
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            // A concurrent start may have won; return the one authenticated daemon.
            if let Some(client) = DaemonClient::discover(cache).await? {
                return Ok(client);
            }
            if let Some(status) = child.try_wait()? {
                bail!("daemon exited ({status}); inspect {}", log_path.display());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    match ready {
        Ok(Ok(client)) => Ok(client),
        result => {
            let _ = child.kill();
            let _ = child.wait();
            match result {
                Ok(Err(error)) => Err(error),
                Err(_) => bail!("daemon startup timed out; inspect {}", log_path.display()),
                Ok(Ok(_)) => unreachable!(),
            }
        }
    }
}
