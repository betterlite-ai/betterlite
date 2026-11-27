mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const MICROSOFT_CLIENT_SECRET: &str = "MICROSOFT_CLIENT_SECRET";
pub const MICROSOFT_TENANT: &str = "MICROSOFT_TENANT";
// ast-grep-ignore: no-long-names - the constant carries LiteLLM's variable name and the client names it in an error
pub const MICROSOFT_AUTHORIZATION_ENDPOINT: &str = "MICROSOFT_AUTHORIZATION_ENDPOINT";
pub const MICROSOFT_TOKEN_ENDPOINT: &str = "MICROSOFT_TOKEN_ENDPOINT";
pub const MICROSOFT_USERINFO_ENDPOINT: &str = "MICROSOFT_USERINFO_ENDPOINT";

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
            client_secret: lookup
                .lookup(MICROSOFT_CLIENT_SECRET)
                .map(SecretString::from),
            tenant: lookup.lookup(MICROSOFT_TENANT),
            authorization_endpoint: lookup.lookup(MICROSOFT_AUTHORIZATION_ENDPOINT),
            token_endpoint: lookup.lookup(MICROSOFT_TOKEN_ENDPOINT),
            userinfo_endpoint: lookup.lookup(MICROSOFT_USERINFO_ENDPOINT),
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
