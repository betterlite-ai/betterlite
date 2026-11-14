use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::providers::copilot::chat as environment;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    disable_copilot_system_to_assistant: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub disable_system_to_assistant: bool,
    pub api_base: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(environment::resolve(
            lookup,
            self.litellm_settings.disable_copilot_system_to_assistant,
        ))
    }
}

mod tests;
