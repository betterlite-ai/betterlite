use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    turn_off_message_logging: Option<bool>,
    redact_user_api_key_info: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub turn_off_message_logging: bool,
    pub redact_user_api_key_info: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            turn_off_message_logging: self
                .litellm_settings
                .turn_off_message_logging
                .unwrap_or(false),
            redact_user_api_key_info: self
                .litellm_settings
                .redact_user_api_key_info
                .unwrap_or(false),
        })
    }
}

#[path = "logging_privacy/tests.rs"]
mod tests;
