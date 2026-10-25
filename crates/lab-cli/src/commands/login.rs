//! Browser/device approval stores only the public laboratory binding.
use anyhow::{ensure, Context};
use osdl_server::account::{channel_for_origin, Binding};
use serde::Deserialize;
use std::time::Duration;

#[derive(clap::Args)]
pub struct LoginArgs {
    /// Exact primary HTTPS origin for this account environment.
    #[arg(long)]
    pub server: String,
}
// No Debug: the private device code must never reach diagnostics.
#[derive(Deserialize)]
struct Challenge {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum PollResponse {
    Binding(Binding),
    Pending { state: PendingState },
}
#[derive(Deserialize)]
enum PendingState {
    #[serde(rename = "authorization_pending")]
    Pending,
}
const API: &str = "/liyanlabs/api/v1/osdl/auth";

pub async fn run(args: LoginArgs, instance: Option<String>) -> anyhow::Result<()> {
    let instance = instance.unwrap_or_else(|| "default".into());
    let paths = osdl_server::Paths::discover().map_err(anyhow::Error::msg)?;
    let channel = channel_for_origin(&args.server)?;
    let path = Binding::path(&paths.config_dir, &instance)?;
    ensure!(
        !path.try_exists()?,
        "instance already bound; reuse its existing laboratory identity"
    );
    // Native trust roots include the user's local Dev CA; redirects never carry a private code.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .context("initialize HTTPS client")?;
    let challenge = begin(&client, &args.server, &instance).await?;
    println!(
        "Open {} and enter code {}",
        challenge.verification_uri, challenge.user_code
    );
    let authorization = async {
        loop {
            tokio::time::sleep(Duration::from_secs(challenge.interval)).await;
            if let Some(binding) = poll(
                &client,
                &format!("{}{API}/device/token", args.server),
                &challenge.device_code,
            )
            .await?
            {
                ensure!(
                    binding.issuer == args.server
                        && binding.channel == channel
                        && binding.instance == instance,
                    "approved binding does not match requested laboratory environment"
                );
                binding.save_new(&paths.config_dir)?;
                return Ok::<(), anyhow::Error>(());
            }
        }
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(challenge.expires_in), authorization) => result.context("authorization expired")?,
        _ = tokio::signal::ctrl_c() => anyhow::bail!("authorization cancelled"),
    }
}
async fn begin(
    client: &reqwest::Client,
    origin: &str,
    instance: &str,
) -> anyhow::Result<Challenge> {
    let response = client
        .post(format!("{origin}{API}/device"))
        .json(&serde_json::json!({"instance":instance}))
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("authorization server unavailable"))?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "device authorization could not start"
    );
    let challenge: Challenge = read_json(response).await?;
    ensure!(
        challenge.device_code.len() == 43
            && challenge
                .device_code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
        "invalid authorization challenge"
    );
    ensure!(
        challenge.user_code.len() == 9
            && challenge
                .user_code
                .bytes()
                .enumerate()
                .all(|(i, b)| if i == 4 {
                    b == b'-'
                } else {
                    b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b)
                }),
        "invalid public verification code"
    );
    ensure!(
        challenge.verification_uri == format!("{origin}/#/osdl/authorize")
            && challenge.verification_uri_complete
                == format!(
                    "{}?user_code={}",
                    challenge.verification_uri, challenge.user_code
                ),
        "verification URL does not match server origin"
    );
    ensure!(
        challenge.interval >= 5
            && challenge.interval <= 60
            && challenge.expires_in > 0
            && challenge.expires_in <= 600,
        "invalid authorization lifetime"
    );
    Ok(challenge)
}
async fn poll(client: &reqwest::Client, url: &str, code: &str) -> anyhow::Result<Option<Binding>> {
    let response = client
        .post(url)
        .json(&serde_json::json!({"device_code":code}))
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("authorization server unavailable"))?;
    match response.status().as_u16() {
        200 => match read_json::<PollResponse>(response).await? {
            PollResponse::Binding(binding) => {
                binding.validate()?;
                Ok(Some(binding))
            }
            PollResponse::Pending {
                state: PendingState::Pending,
            } => Ok(None),
        },
        429 => Ok(None),
        403 => anyhow::bail!("authorization denied"),
        410 => anyhow::bail!("authorization expired"),
        409 => anyhow::bail!("authorization already consumed"),
        _ => anyhow::bail!("authorization request failed"),
    }
}
async fn read_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> anyhow::Result<T> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("invalid authorization response"))?
    {
        ensure!(
            body.len() + chunk.len() <= 8192,
            "authorization response too large"
        );
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| anyhow::anyhow!("invalid authorization response"))
}
#[cfg(test)]
mod tests;
