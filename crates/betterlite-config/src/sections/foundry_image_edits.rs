mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use serde::Deserialize;

pub const API_VERSION_VAR: &str = "AZURE_AI_API_VERSION";

// AZURE_AI_API_VERSION comes from the environment only, never litellm_settings or a deployment's litellm_params
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_version: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_version: lookup
                .lookup(API_VERSION_VAR)
                .filter(|value| !value.is_empty()),
        })
    }
}
