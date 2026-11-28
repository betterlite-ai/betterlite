mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const CLIENT_SECRET_ENV: &str = "MICROSOFT_CLIENT_SECRET";
pub const TENANT_ENV: &str = "MICROSOFT_TENANT";
pub const AUTHORIZATION_ENDPOINT_ENV: &str = "MICROSOFT_AUTHORIZATION_ENDPOINT";
pub const TOKEN_ENDPOINT_ENV: &str = "MICROSOFT_TOKEN_ENDPOINT";
pub const USERINFO_ENDPOINT_ENV: &str = "MICROSOFT_USERINFO_ENDPOINT";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub client_secret: Option<SecretString>,
    pub tenant: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub userinfo_endpoint: Option<String>,
}

impl Settings {
    // LiteLLM tests the secret and tenant with `is None`, so an empty value stays set and the client falls back from it
    #[must_use]
    pub fn resolve_environment<L>(lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            client_secret: lookup.lookup(CLIENT_SECRET_ENV).map(SecretString::from),
            tenant: lookup.lookup(TENANT_ENV),
            authorization_endpoint: lookup.lookup(AUTHORIZATION_ENDPOINT_ENV),
            token_endpoint: lookup.lookup(TOKEN_ENDPOINT_ENV),
            userinfo_endpoint: lookup.lookup(USERINFO_ENDPOINT_ENV),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings::resolve_environment(lookup))
    }
}
