#[path = "verbose_hooks/tests.rs"]
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
    #[serde(default = "default_set_verbose")]
    pub set_verbose: JsonValue,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            set_verbose: default_set_verbose(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub set_verbose: JsonValue,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            set_verbose: default_set_verbose(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            set_verbose: resolve_setting(self.litellm_settings.set_verbose, lookup)?,
        })
    }
}

const fn default_set_verbose() -> JsonValue {
    JsonValue::Bool(false)
}

fn resolve_setting<L>(value: JsonValue, lookup: &L) -> Result<JsonValue, ConfigError>
where
    L: EnvironmentLookup,
{
    let JsonValue::String(text) = value else {
        return Ok(value);
    };
    resolve_string(text, "litellm_settings.set_verbose", lookup).map(JsonValue::String)
}
