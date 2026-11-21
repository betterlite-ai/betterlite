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
    pub skip_system_message_in_guardrail: Option<bool>,
    #[serde(default)]
    pub skip_tool_message_in_guardrail: Option<bool>,
}

// LiteLLM's module defaults, used only where a guardrail leaves its own skip flag unset
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub skip_system_message: bool,
    pub skip_tool_message: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            skip_system_message: self
                .settings
                .skip_system_message_in_guardrail
                .unwrap_or(false),
            skip_tool_message: self
                .settings
                .skip_tool_message_in_guardrail
                .unwrap_or(false),
        })
    }
}
