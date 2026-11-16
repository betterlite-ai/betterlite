use betterlite_secret_managers::secret_lookup::text_to_bool;
use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const URL_VARIABLE: &str = "VIGIL_GUARD_URL";
pub const API_KEY_VARIABLE: &str = "VIGIL_GUARD_API_KEY";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: lookup
                .lookup(URL_VARIABLE)
                .filter(|value| !value.is_empty() && text_to_bool(value).is_none()),
            api_key: lookup
                .lookup(API_KEY_VARIABLE)
                .filter(|value| !value.is_empty() && text_to_bool(value).is_none())
                .map(SecretString::from),
        })
    }
}

#[path = "vigil_runtime/tests.rs"]
mod tests;
