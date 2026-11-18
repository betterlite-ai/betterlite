mod tests;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const REASONING_AUTO_SUMMARY_ENV: &str = "LITELLM_REASONING_AUTO_SUMMARY";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    #[serde(default)]
    reasoning_auto_summary: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub reasoning_auto_summary: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM ORs the global with a lowercased, untrimmed "true", so the environment only ever turns it on
        let environment = lookup
            .lookup(REASONING_AUTO_SUMMARY_ENV)
            .is_some_and(|value| value.to_lowercase() == "true");
        Ok(Settings {
            reasoning_auto_summary: self.litellm_settings.reasoning_auto_summary == Some(true)
                || environment,
        })
    }
}
