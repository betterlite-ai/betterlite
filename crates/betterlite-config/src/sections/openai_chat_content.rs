use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
    #[serde(default)]
    pub openai_system_messages_first: Option<bool>,
}

// LiteLLM reads this module-level flag from litellm_settings only; no environment variable sets it
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub openai_system_messages_first: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            openai_system_messages_first: self
                .settings
                .openai_system_messages_first
                .unwrap_or_default(),
        })
    }
}
