use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub gateway: GatewayConfig,
    pub surrealdb: SurrealConfig,
    pub storage: StorageConfig,
    pub google: GoogleConfig,
    pub smtp: SmtpConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub bind: String,
    pub frontend_origin: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurrealConfig {
    pub endpoint: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

/// Outbound email transport. The gateway uses this to send invite emails.
/// `app_base_url` is the public origin embedded in transactional links
/// (e.g. https://sharam.example.com). Trailing slash is trimmed at use.
#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from_email: String,
    pub from_name: String,
    pub app_base_url: String,
    /// "starttls" (587), "tls" (465), or "plain" (local dev / mailhog).
    #[serde(default = "default_encryption")]
    pub encryption: String,
}

fn default_encryption() -> String {
    "starttls".to_string()
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();

        Figment::new()
            .merge(Toml::file("Sharam.toml"))
            .merge(Env::prefixed("SHARAM_").split("__"))
            .extract()
            .map_err(|e| Error::Config(e.to_string()))
    }
}
