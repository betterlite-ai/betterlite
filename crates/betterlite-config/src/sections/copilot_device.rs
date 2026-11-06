use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::copilot::device as environment;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub token_dir: String,
    pub access_token_file: String,
    pub api_key_file: String,
    pub client_id: String,
    pub device_code_url: String,
    pub access_token_url: String,
    pub api_key_url: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(environment::resolve(lookup))
    }
}

mod tests;
