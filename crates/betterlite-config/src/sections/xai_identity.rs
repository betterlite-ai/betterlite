#[path = "xai_identity/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    xai_key: Option<SecretString>,
    api_key: Option<SecretString>,
}

// an absent base is a real state: the chat family falls back to a different default than model discovery
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub xai_key: Option<SecretString>,
    pub generic_api_key: Option<SecretString>,
    pub environment_key: Option<SecretString>,
    pub api_base: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = self.litellm_settings;
        Ok(Settings {
            xai_key: resolve_secret(module.xai_key, "litellm_settings.xai_key", lookup)?,
            generic_api_key: resolve_secret(module.api_key, "litellm_settings.api_key", lookup)?,
            environment_key: lookup
                .lookup("XAI_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            api_base: lookup
                .lookup("XAI_API_BASE")
                .filter(|value| !value.is_empty()),
        })
    }
}
