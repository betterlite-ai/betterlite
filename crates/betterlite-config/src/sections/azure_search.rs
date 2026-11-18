use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const PROJECT_ENDPOINT_ENV: &str = "BING_GROUNDING_PROJECT_ENDPOINT";
pub const MODEL_ENV: &str = "BING_GROUNDING_MODEL";
pub const CONNECTION_ID_ENV: &str = "BING_GROUNDING_CONNECTION_ID";
pub const TOKEN_ENV: &str = "BING_GROUNDING_TOKEN";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub project_endpoint: Option<String>,
    pub model: Option<String>,
    pub connection_id: Option<String>,
    pub token: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            project_endpoint: lookup
                .lookup(PROJECT_ENDPOINT_ENV)
                .filter(|value| !value.is_empty()),
            model: lookup.lookup(MODEL_ENV).filter(|value| !value.is_empty()),
            connection_id: lookup
                .lookup(CONNECTION_ID_ENV)
                .filter(|value| !value.is_empty()),
            token: lookup
                .lookup(TOKEN_ENV)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}
