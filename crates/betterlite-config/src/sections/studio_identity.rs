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

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub environment_key: Option<SecretString>,
    pub generic_api_key: Option<SecretString>,
    pub api_base: Option<String>,
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
                .filter(|value| !value.is_empty()),
        })
    }
}
