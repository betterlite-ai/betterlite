use secrecy::{ExposeSecret, SecretString};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const REDACTED: &str = "[redacted]";
const API_BASE_VARIABLE: &str = "AZURE_SPEECH_API_BASE";
const API_KEY_VARIABLE: &str = "AZURE_SPEECH_API_KEY";

// a map-shaped Raw, not a unit struct, so serde still accepts (and ignores) the document's keys
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// LiteLLM reads both through get_secret_str: the environment or a secret manager, never a litellm_settings key
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

impl Settings {
    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        self.api_key
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .filter(|value| !value.is_empty())
            .map_or_else(|| text.to_owned(), |value| text.replace(value, REDACTED))
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|value| !value.is_empty()),
            api_key: lookup
                .lookup(API_KEY_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}
