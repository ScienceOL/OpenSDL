use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use osdl_proto::v1 as pb;

#[derive(Subcommand)]
pub enum AccountCommand {
    /// List the bound owner and explicitly admitted LiyanLabs accounts.
    List,
    /// Grant or replace an exact LiyanLabs account subject's laboratory role.
    Grant {
        subject: String,
        #[arg(long, value_enum)]
        role: AccountRole,
    },
    /// Revoke a LiyanLabs account subject's laboratory access.
    Revoke { subject: String },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum AccountRole {
    Viewer,
    Operator,
    Admin,
}

impl From<AccountRole> for pb::AccountRole {
    fn from(role: AccountRole) -> Self {
        match role {
            AccountRole::Viewer => Self::Viewer,
            AccountRole::Operator => Self::Operator,
            AccountRole::Admin => Self::Admin,
        }
    }
}

pub async fn admin(cmd: AccountCommand, opts: crate::client::ClientOpts) -> Result<()> {
    ensure!(
        opts.endpoint.is_none() && opts.connection.is_none() && opts.auth_token.is_none(),
        "manage account roles on the laboratory host using --instance"
    );
    let instance = opts
        .instance
        .context("account administration requires --instance")?;
    let (channel, interceptor) = crate::client::local_admin(Some(instance)).await?;
    let mut client = pb::account_membership_administration_client::AccountMembershipAdministrationClient::with_interceptor(channel, interceptor);
    match cmd {
        AccountCommand::List => {
            let mut after_subject = String::new();
            loop {
                let first_page = after_subject.is_empty();
                let page = client
                    .list_members(pb::ListAccountMembersRequest { after_subject })
                    .await?
                    .into_inner();
                if first_page {
                    println!("{}\towner", page.owner_subject);
                }
                for member in page.members {
                    let role = member
                        .role()
                        .as_str_name()
                        .trim_start_matches("ACCOUNT_ROLE_")
                        .to_ascii_lowercase();
                    println!("{}\t{role}", member.subject);
                }
                after_subject = page.next_subject;
                if after_subject.is_empty() {
                    break;
                }
            }
        }
        AccountCommand::Grant { subject, role } => {
            client
                .grant(pb::GrantAccountMemberRequest {
                    subject,
                    role: pb::AccountRole::from(role) as i32,
                })
                .await?;
            println!("Account access granted.");
        }
        AccountCommand::Revoke { subject } => {
            client
                .revoke(pb::RevokeAccountMemberRequest { subject })
                .await?;
            println!("Account access revoked. Active subscriptions released.");
        }
    }
    Ok(())
}
