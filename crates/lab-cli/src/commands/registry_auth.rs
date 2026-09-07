use clap::Args;
use opensdl_assets::RegistryCredentials;

#[derive(Args)]
pub struct RegistryAuthArgs {
    /// Registry username. Must be provided with --password.
    #[arg(
        long,
        env = "LAB_REGISTRY_USERNAME",
        requires = "password",
        conflicts_with = "token"
    )]
    username: Option<String>,
    /// Registry password. Prefer LAB_REGISTRY_PASSWORD over shell history.
    #[arg(
        long,
        env = "LAB_REGISTRY_PASSWORD",
        requires = "username",
        conflicts_with = "token",
        hide_env_values = true
    )]
    password: Option<String>,
    /// Registry bearer token. Prefer LAB_REGISTRY_TOKEN over shell history.
    #[arg(
        long,
        env = "LAB_REGISTRY_TOKEN",
        conflicts_with_all = ["username", "password"],
        hide_env_values = true
    )]
    token: Option<String>,
}

impl RegistryAuthArgs {
    pub fn credentials(&self) -> RegistryCredentials {
        match (&self.token, &self.username, &self.password) {
            (Some(token), _, _) => RegistryCredentials::Bearer(token.clone()),
            (None, Some(username), Some(password)) => RegistryCredentials::Basic {
                username: username.clone(),
                password: password.clone(),
            },
            _ => RegistryCredentials::Anonymous,
        }
    }
}
