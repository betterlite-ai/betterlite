use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const DEFAULT_SUMMARY_MAX_TOKENS: u64 = 4096;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    general_settings: RawGeneralSettingsInput,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum RawGeneralSettingsInput {
    #[default]
    Null,
    Settings(RawGeneralSettings),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawGeneralSettings {
    // ast-grep-ignore: no-long-names
    context_management_summary_model: JsonValue,
    // ast-grep-ignore: no-long-names
    context_management_summary_max_tokens: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    // ast-grep-ignore: no-long-names
    pub context_management_summary_model: Option<String>,
    // ast-grep-ignore: no-long-names
    pub context_management_summary_max_tokens: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // ast-grep-ignore: no-long-names
            context_management_summary_model: None,
            // ast-grep-ignore: no-long-names
            context_management_summary_max_tokens: DEFAULT_SUMMARY_MAX_TOKENS,
        }
    }
}

// _read_summary_model_setting keeps a nonempty string as sent, whitespace included
fn summary_model(value: JsonValue) -> Option<String> {
    match value {
        JsonValue::String(model) if !model.is_empty() => Some(model),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)
        | JsonValue::String(_) => None,
    }
}

// _read_summary_max_tokens_setting counts a positive int, and Python's True is the int 1
fn summary_max_tokens(value: JsonValue) -> u64 {
    match value {
        JsonValue::Number(number) => number
            .as_u64()
            .filter(|tokens| *tokens > 0)
            .unwrap_or(DEFAULT_SUMMARY_MAX_TOKENS),
        JsonValue::Bool(true) => 1,
        JsonValue::Null
        | JsonValue::Bool(false)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => DEFAULT_SUMMARY_MAX_TOKENS,
    }
}

impl Section for Raw {
    type Settings = Settings;

    // neither reader consults the environment: DEFAULT_MAX_TOKENS is not a fallback here
    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general_settings = match self.general_settings {
            RawGeneralSettingsInput::Null => RawGeneralSettings::default(),
            RawGeneralSettingsInput::Settings(settings) => settings,
        };
        Ok(Settings {
            // ast-grep-ignore: no-long-names
            context_management_summary_model: summary_model(
                // ast-grep-ignore: no-long-names
                general_settings.context_management_summary_model,
            ),
            // ast-grep-ignore: no-long-names
            context_management_summary_max_tokens: summary_max_tokens(
                // ast-grep-ignore: no-long-names
                general_settings.context_management_summary_max_tokens,
            ),
        })
    }
}

#[path = "summary_model_access/tests.rs"]
mod tests;
