//! Application settings from `config.toml`. Secrets are not part of it: they
//! stay in the environment (`.env`) and are read with [`secret`].
//!
//! The file is `AEZA_CONFIG`, or `config.toml` in the working directory.
//! Unknown keys are errors, so a typo does not silently fall back to a default.

use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    pub auth: Auth,
    #[serde(default)]
    pub demo: Demo,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    /// The address users open, e.g. `https://127.0.0.1:8080`. With https://
    /// the frontend serves HTTPS and the API marks its cookies Secure.
    pub public_url: String,
    #[serde(default = "default_session_hours")]
    pub session_hours: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub mode: AuthMode,
    #[serde(default)]
    pub default_user: DefaultUser,
    pub oidc: Option<Oidc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// Emails and Argon2 password hashes in public.admins.
    Password,
    /// An OpenID provider proves the email; public.admins grants access.
    Oidc,
    /// Every request acts as a superadmin. Development only.
    Disabled,
}

/// With `mode = "password"`: a superadmin created at startup while no admin
/// has a password. Its password is the DEFAULT_USER_PASSWORD secret.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultUser {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub login: String,
}

/// The client secret, if the client has one, is the OIDC_CLIENT_SECRET secret.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Oidc {
    pub issuer: String,
    pub client_id: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}

/// The synthetic customer stream of the demo-stream Compose profile.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Demo {
    pub interval_seconds: u64,
    /// Virtual days of history replayed before live events; 0 skips it.
    pub replay_days: u32,
    pub replay_delay_ms: u64,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            interval_seconds: 15,
            replay_days: 60,
            replay_delay_ms: 200,
        }
    }
}

fn default_session_hours() -> i32 {
    12
}

fn default_scopes() -> Vec<String> {
    ["openid", "email", "profile"].map(String::from).to_vec()
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = std::env::var_os("AEZA_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| "config.toml".into());
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read the configuration {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid configuration {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.server.public_url)
            .context("server.public_url is not a URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.path() == "/" && url.query().is_none(),
            "server.public_url must be an http:// or https:// origin without a path, e.g. https://127.0.0.1:8080"
        );
        ensure!(
            (1..=720).contains(&self.server.session_hours),
            "server.session_hours must be between 1 and 720"
        );
        if self.auth.mode == AuthMode::Oidc {
            ensure!(
                self.auth.oidc.is_some(),
                "auth.mode = \"oidc\" requires an [auth.oidc] section"
            );
        }
        if self.auth.default_user.enabled {
            ensure!(
                self.auth.mode == AuthMode::Password,
                "auth.default_user requires auth.mode = \"password\""
            );
            ensure!(
                !self.auth.default_user.login.is_empty(),
                "auth.default_user.login is empty"
            );
        }
        let demo = &self.demo;
        ensure!(
            (2..=3600).contains(&demo.interval_seconds),
            "demo.interval_seconds must be 2–3600"
        );
        ensure!(
            demo.replay_days == 0 || (31..=365).contains(&demo.replay_days),
            "demo.replay_days must be 0 or 31–365"
        );
        ensure!(
            demo.replay_delay_ms <= 10_000,
            "demo.replay_delay_ms must be at most 10000"
        );
        Ok(())
    }

    /// `server.public_url` without the trailing slash, as browsers send it in Origin.
    pub fn origin(&self) -> &str {
        self.server.public_url.trim_end_matches('/')
    }
}

/// A secret from the environment; unset and empty are the same, since
/// Compose passes unset variables as empty strings.
pub fn secret(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
        [server]
        public_url = "https://127.0.0.1:8080"
        [auth]
        mode = "password"
    "#;

    fn error(text: &str) -> String {
        format!("{:#}", Config::parse(text).unwrap_err())
    }

    #[test]
    fn the_committed_config_is_valid_and_has_no_secrets() {
        let text = include_str!("../config.toml");
        Config::parse(text).unwrap();
        for line in text.lines().map(str::trim).filter(|line| !line.starts_with('#')) {
            let key = line.split('=').next().unwrap_or_default().to_lowercase();
            assert!(
                !["password", "secret", "token", "api_key"].iter().any(|word| key.contains(word)),
                "secrets belong in .env, not config.toml: {line}"
            );
        }
    }

    #[test]
    fn defaults_fill_what_is_left_out() {
        let config = Config::parse(MINIMAL).unwrap();
        assert_eq!(config.server.session_hours, 12);
        assert_eq!(config.origin(), "https://127.0.0.1:8080");
        assert!(!config.auth.default_user.enabled);
        assert_eq!(config.demo.replay_days, 60);
    }

    #[test]
    fn mistakes_are_reported_instead_of_ignored() {
        assert!(error(&MINIMAL.replace("mode", "mod")).contains("unknown field `mod`"));
        assert!(error(&MINIMAL.replace("password", "passwd")).contains("unknown variant `passwd`"));
        assert!(error(&MINIMAL.replace("8080\"", "8080/app\"")).contains("without a path"));
        assert!(error(&MINIMAL.replace("https://", "ftp://")).contains("http:// or https://"));
        assert!(error(&MINIMAL.replace("password", "oidc")).contains("requires an [auth.oidc] section"));
        let default_user = format!("{MINIMAL}\n[auth.default_user]\nenabled = true\n");
        assert!(error(&default_user).contains("login is empty"));
        let with_oidc = format!("{default_user}login = \"admin\"\n").replace("\"password\"", "\"disabled\"");
        assert!(error(&with_oidc).contains("requires auth.mode = \"password\""));
        assert!(error(&format!("{MINIMAL}\n[demo]\nreplay_days = 7\n")).contains("0 or 31–365"));
    }
}
