mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const API_KEY_VARIABLE: &str = "XECGUARD_API_KEY";
pub const API_BASE_VARIABLE: &str = "XECGUARD_API_BASE";
pub const BLOCK_ON_ERROR_VARIABLE: &str = "XECGUARD_BLOCK_ON_ERROR";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub block_on_error: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM reads these with os.environ, where an empty value is as good as unset
        Ok(Settings {
            api_key: lookup
                .lookup(API_KEY_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            api_base: lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|value| !value.is_empty()),
            block_on_error: lookup
                .lookup(BLOCK_ON_ERROR_VARIABLE)
                .filter(|value| !value.is_empty()),
        })
    }
}
