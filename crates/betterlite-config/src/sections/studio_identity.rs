mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    api_key: Option<SecretString>,
}

pub const DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com";

#[derive(Debug, Clone)]
pub struct Settings {
    pub environment_key: Option<SecretString>,
    pub generic_api_key: Option<SecretString>,
    pub api_base: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            environment_key: None,
            generic_api_key: None,
            api_base: DEFAULT_API_BASE.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            environment_key: lookup
                .lookup("GOOGLE_API_KEY")
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    lookup
                        .lookup("GEMINI_API_KEY")
                        .filter(|value| !value.is_empty())
                })
                .map(SecretString::from),
            generic_api_key: resolve_secret(
                self.litellm_settings.api_key,
                "litellm_settings.api_key",
                lookup,
            )?,
            api_base: lookup
                .lookup("GEMINI_API_BASE")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        })
    }
}
