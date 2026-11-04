#[path = "vector_context_shaping/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, serde::Deserialize)]
pub struct Global {
    #[serde(rename = "vector_store_search_failure_mode", default = "default_mode")]
    pub failure_mode: JsonValue,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            failure_mode: default_mode(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub failure_mode: JsonValue,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            failure_mode: default_mode(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let failure_mode = match self.litellm_settings.failure_mode {
            JsonValue::String(text) => JsonValue::String(resolve_string(
                text,
                "litellm_settings.vector_store_search_failure_mode",
                lookup,
            )?),
            value @ (JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_)) => value,
        };
        Ok(Settings { failure_mode })
    }
}

fn default_mode() -> JsonValue {
    JsonValue::String("annotate".to_owned())
}
