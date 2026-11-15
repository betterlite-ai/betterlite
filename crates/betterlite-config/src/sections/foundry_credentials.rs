mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const API_BASE_VAR: &str = "AZURE_AI_API_BASE";
pub const API_KEY_VAR: &str = "AZURE_AI_API_KEY";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

impl Settings {
    #[must_use]
    pub fn resolve_environment<L>(lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            api_base: lookup.lookup(API_BASE_VAR),
            api_key: lookup.lookup(API_KEY_VAR).map(SecretString::from),
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
