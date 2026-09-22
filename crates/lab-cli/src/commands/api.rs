//! Account API commands. Credentials are scoped to OpenSDL read access and
//! kept separate from local gRPC and OCI registry authentication.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use clap::{Args, Subcommand};
use osdl_server::paths::Paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Subcommand)]
pub enum ApiCmd {
    /// Save an account API token after checking it with the server.
    Login(LoginArgs),
    /// Remove the saved account API token from this machine.
    Logout,
    /// Show online OpenSDL devices on your connected runners.
    Devices(DevicesArgs),
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Server origin, e.g. https://scilaxy.ai (without an API path).
    #[arg(long)]
    server: String,
    /// Read the token from stdin instead of a hidden terminal prompt.
    #[arg(long)]
    token_stdin: bool,
}

#[derive(Debug, Args)]
pub struct DevicesArgs {
    /// Emit the response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    server: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct Runner {
    runner_id: String,
    name: String,
    online: bool,
    devices: Vec<Device>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Device {
    device_id: String,
    device_type: String,
    role: Option<String>,
    online: bool,
}

#[derive(Debug, Deserialize)]
struct DeviceResponse {
    runners: Vec<Runner>,
}

pub async fn run(cmd: ApiCmd) -> anyhow::Result<()> {
    let path = credentials_path()?;
    match cmd {
        ApiCmd::Login(args) => login(args, &path).await,
        ApiCmd::Logout => {
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
            println!("Account API token removed from this machine.");
            Ok(())
        }
        ApiCmd::Devices(args) => devices(args, &path).await,
    }
}

async fn login(args: LoginArgs, path: &Path) -> anyhow::Result<()> {
    let server = parse_server(&args.server)?;
    let token = if args.token_stdin {
        let mut value = String::new();
        io::stdin().take(256).read_to_string(&mut value)?;
        value.trim().to_owned()
    } else {
        rpassword::prompt_password("OpenSDL API token: ")?
    };
    if !token.starts_with("sdl_") || token.len() != 47 {
        bail!("invalid OpenSDL API token format");
    }
    let client = http_client()?;
    let response = client
        .get(endpoint(&server, "me")?)
        .bearer_auth(&token)
        .send()
        .await
        .context("verify account API token")?;
    if !response.status().is_success() {
        bail!("API login failed: HTTP {}", response.status());
    }
    save_credentials(path, &Credentials { server, token })?;
    println!("Authenticated to SciLaxy. Run `lab api devices` to view online devices.");
    Ok(())
}

async fn devices(args: DevicesArgs, path: &Path) -> anyhow::Result<()> {
    let credentials = read_credentials(path)?;
    let response = http_client()?
        .get(endpoint(&credentials.server, "devices")?)
        .bearer_auth(&credentials.token)
        .send()
        .await
        .context("query account devices")?;
    if !response.status().is_success() {
        bail!("device query failed: HTTP {}", response.status());
    }
    let body = response.bytes().await.context("read device list")?;
    if args.json {
        let value: serde_json::Value =
            serde_json::from_slice(&body).context("decode device list")?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let response: DeviceResponse = serde_json::from_slice(&body).context("decode device list")?;
    for runner in response.runners {
        println!(
            "{} ({}) — {}",
            runner.name,
            runner.runner_id,
            if runner.online { "online" } else { "offline" }
        );
        if let Some(error) = runner.error {
            println!("  {error}");
        }
        for device in runner.devices {
            if device.online {
                println!(
                    "  {}  {}  {}",
                    device.device_id,
                    device.device_type,
                    device.role.as_deref().unwrap_or("-")
                );
            }
        }
    }
    Ok(())
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?)
}

fn parse_server(raw: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(raw).context("invalid server URL")?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "scilaxy.local")
    );
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        bail!("server must use HTTPS (HTTP is allowed only for local development)");
    }
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("server must be an origin without credentials, path, query, or fragment");
    }
    Ok(url.origin().ascii_serialization())
}

fn endpoint(server: &str, resource: &str) -> anyhow::Result<reqwest::Url> {
    Ok(reqwest::Url::parse(server)?.join(&format!("/liyanlabs/api/v1/osdl/{resource}"))?)
}

fn credentials_path() -> anyhow::Result<PathBuf> {
    let paths = Paths::discover().map_err(anyhow::Error::msg)?;
    Ok(paths.config_dir.join("account-api.json"))
}

fn read_credentials(path: &Path) -> anyhow::Result<Credentials> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "no saved account token; run `lab api login --server URL` ({})",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes).context("invalid saved account API credentials")
}

fn save_credentials(path: &Path, credentials: &Credentials) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("account API config has no parent directory")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create credential file in {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec(credentials)?)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_must_be_a_secure_origin() {
        assert_eq!(
            parse_server("https://example.org/").unwrap(),
            "https://example.org"
        );
        for raw in [
            "http://example.org",
            "https://example.org/api",
            "https://user:pass@example.org",
            "https://example.org?token=secret",
        ] {
            assert!(parse_server(raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn saved_credentials_are_private_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account-api.json");
        let credentials = Credentials {
            server: "https://example.org".into(),
            token: "sdl_test".into(),
        };
        save_credentials(&path, &credentials).unwrap();
        let loaded = read_credentials(&path).unwrap();
        assert_eq!(loaded.server, credentials.server);
        assert_eq!(loaded.token, credentials.token);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
