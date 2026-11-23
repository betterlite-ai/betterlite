mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub openai_api_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: self
                .settings
                .api_base
                .map(|text| resolve_string(text, "litellm_settings.api_base", lookup))
                .transpose()?,
            api_key: resolve_secret(self.settings.api_key, "litellm_settings.api_key", lookup)?,
            openai_key: resolve_secret(
                self.settings.openai_key,
                "litellm_settings.openai_key",
                lookup,
            )?,
            openai_api_key: lookup
                .lookup("OPENAI_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}
