use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawSettings {
    #[serde(rename = "disable_anthropic_gemini_context_caching_transform")]
    disable_context_caching: bool,
}

// LiteLLM copies litellm_settings onto the litellm module, and is_cached_message reads the flag on every cache plan
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub disable_context_caching: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            disable_context_caching: self.litellm_settings.disable_context_caching,
        })
    }
}
