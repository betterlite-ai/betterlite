mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const LOGGED_EVENT_TYPES_PATH: &str = "litellm_settings.logged_real_time_event_types";
const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Global {
    #[serde(default)]
    pub logged_real_time_event_types: JsonValue,
}

// LiteLLM's litellm.logged_real_time_event_types is None, a list of event names or "*", and the connection applies its own five defaults for None
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LoggedEventTypes {
    #[default]
    Default,
    All,
    Only(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub logged_real_time_event_types: LoggedEventTypes,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            logged_real_time_event_types: logged_event_types(
                self.litellm_settings.logged_real_time_event_types,
                lookup,
            )?,
        })
    }
}

fn logged_event_types<L>(value: JsonValue, lookup: &L) -> Result<LoggedEventTypes, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        JsonValue::Null => Ok(LoggedEventTypes::Default),
        JsonValue::String(text) if text == "*" => Ok(LoggedEventTypes::All),
        JsonValue::String(text) if text.starts_with(ENVIRONMENT_PREFIX) => {
            environment_selection(text, lookup)
        }
        JsonValue::Array(items) => {
            let mut types = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    JsonValue::String(event) => {
                        types.push(resolve_string(event, LOGGED_EVENT_TYPES_PATH, lookup)?);
                    }
                    other => return Err(invalid_scalar(&other)),
                }
            }
            Ok(LoggedEventTypes::Only(types))
        }
        other => Err(invalid_scalar(&other)),
    }
}

// LiteLLM's proxy substitutes os.environ/ references in every value before it assigns the setting; an empty variable counts as unset
fn environment_selection<L>(reference: String, lookup: &L) -> Result<LoggedEventTypes, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = resolve_string(reference, LOGGED_EVENT_TYPES_PATH, lookup)?;
    if resolved == "*" {
        return Ok(LoggedEventTypes::All);
    }
    if resolved.is_empty() {
        return Ok(LoggedEventTypes::Default);
    }
    Err(invalid_scalar(&JsonValue::String(resolved)))
}

fn invalid_scalar(value: &JsonValue) -> ConfigError {
    ConfigError::InvalidScalar {
        path: LOGGED_EVENT_TYPES_PATH.to_owned(),
        value: matches!(value, JsonValue::Bool(true)),
    }
}
