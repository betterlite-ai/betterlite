use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::mistral::options as environment;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub azure_api_base: Option<String>,
    pub azure_api_key: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub max_recurse_depth: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            azure_api_base: None,
            azure_api_key: None,
            api_key: None,
            max_recurse_depth: environment::DEFAULT_MAX_RECURSE_DEPTH,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let options = environment::resolve(lookup)?;
        Ok(Settings {
            azure_api_base: options.azure_api_base,
            azure_api_key: options.azure_api_key,
            api_key: options.api_key,
            max_recurse_depth: options.max_recurse_depth,
        })
    }
}
