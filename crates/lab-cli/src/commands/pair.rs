use anyhow::{ensure, Result};
use clap::{Args, Subcommand};
use osdl_proto::v1 as pb;
use std::io::{IsTerminal, Read, Write};
#[derive(Args)]
pub struct PairArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    label: Option<String>,
    /// Read an invitation from redirected standard input. Never pass it in argv.
    #[arg(long)]
    stdin: bool,
    /// Explicitly replace this local profile (does not revoke the old grant).
    #[arg(long)]
    replace: bool,
    /// Confirm the server-authenticated grant noninteractively.
    #[arg(long)]
    yes: bool,
}
#[derive(Subcommand)]
pub enum AuthCommand {
    Invite {
        #[arg(long,value_parser=["read","operate"])]
        access: Option<String>,
        #[arg(long)]
        valid_for: Option<String>,
    },
    List,
    Revoke {
        id: String,
    },
}
fn prompt(label: &str) -> Result<String> {
    ensure!(
        std::io::stdin().is_terminal(),
        "explicit --access and --valid-for required without terminal"
    );
    print!("{label}");
    std::io::stdout().flush()?;
    let mut value = String::new();
    std::io::stdin().read_line(&mut value)?;
    Ok(value.trim().into())
}
pub async fn pair(args: PairArgs) -> Result<()> {
    let text = if args.stdin {
        let mut value = String::new();
        std::io::stdin().take(8194).read_to_string(&mut value)?;
        value.trim_end_matches(['\r', '\n']).to_string()
    } else {
        rpassword::prompt_password("Invitation (hidden): ")?
    };
    let invitation = osdl_auth::Invitation::parse(&text)?;
    let metadata = osdl_auth::inspect_invitation(&invitation).await?;
    println!(
        "Laboratory {} ({}) / {} / {}\nAccess: {}  Grant deadline: {:?}  Invitation deadline: {}",
        metadata.instance,
        metadata.lab_id,
        metadata.channel,
        invitation.endpoint,
        if metadata.access == 2 {
            "Operate"
        } else {
            "Read"
        },
        metadata.grant_expires_at,
        metadata.invitation_expires_at
    );
    ensure!(
        args.yes || prompt("Pair this computer? [yes/no]: ")? == "yes",
        "pairing_cancelled"
    );
    let grant = crate::client::profile_store()?
        .pair(
            &args.name,
            &invitation,
            args.label.as_deref().unwrap_or(&args.name),
            args.replace,
        )
        .await?;
    println!(
        "Saved connection {}. Grant {}. Certificate deadline {}.",
        args.name, grant.id, grant.certificate_expires_at
    );
    Ok(())
}
pub async fn admin(cmd: AuthCommand, instance: Option<String>) -> Result<()> {
    let (channel, interceptor) = crate::client::local_admin(instance).await?;
    let mut client = pb::grant_administration_client::GrantAdministrationClient::with_interceptor(
        channel,
        interceptor,
    );
    match cmd {
        AuthCommand::Invite { access, valid_for } => {
            let access = match access {
                Some(v) => v,
                None => prompt("Access [read/operate]: ")?,
            };
            let access = match access.as_str() {
                "read" => 1,
                "operate" => 2,
                _ => anyhow::bail!("access must be read or operate"),
            };
            let duration = match valid_for {
                Some(v) => v,
                None => prompt("Grant duration [until-revoked/30d/NUMBERd/NUMBERh]: ")?,
            };
            let expires_at = if duration == "until-revoked" {
                None
            } else {
                let (mult, text) = if let Some(t) = duration.strip_suffix('d') {
                    (86400, t)
                } else if let Some(t) = duration.strip_suffix('h') {
                    (3600, t)
                } else {
                    anyhow::bail!("use until-revoked, NUMBERd or NUMBERh")
                };
                let seconds = text
                    .parse::<u64>()?
                    .checked_mul(mult)
                    .ok_or_else(|| anyhow::anyhow!("duration too large"))?;
                ensure!(
                    seconds > 0 && seconds <= 20 * 365 * 86400,
                    "duration out of range"
                );
                Some((osdl_auth::now() + seconds) as i64)
            };
            let response = client
                .invite(pb::InviteRequest { access, expires_at })
                .await?
                .into_inner();
            println!("{}", response.invitation);
        }
        AuthCommand::List => {
            let mut after_id = String::new();
            loop {
                let page = client
                    .list_grants(pb::ListGrantsRequest {
                        after_id,
                        limit: 100,
                    })
                    .await?
                    .into_inner();
                for g in page.grants {
                    println!(
                        "{}\t{}\t{}\texpires={:?}\tcertificate={}\trevoked={}",
                        g.id, g.label, g.access, g.expires_at, g.certificate_expires_at, g.revoked
                    );
                }
                if page.next_id.is_empty() {
                    break;
                }
                after_id = page.next_id;
            }
        }
        AuthCommand::Revoke { id } => {
            client.revoke(pb::RevokeGrantRequest { id }).await?;
            println!("Grant revoked; active subscriptions released.");
        }
    }
    Ok(())
}
