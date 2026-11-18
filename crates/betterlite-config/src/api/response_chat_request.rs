use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(default)]
    reasoning_auto_summary: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub reasoning_auto_summary: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM ORs the global flag with the environment, so an explicit false cannot turn a true environment off
        let environment = lookup
            .lookup("LITELLM_REASONING_AUTO_SUMMARY")
            .is_some_and(|value| value.to_lowercase() == "true");
        Ok(Settings {
            reasoning_auto_summary: self.litellm_settings.reasoning_auto_summary == Some(true)
                || environment,
        })
    }
}
