mod tests;

use betterlite_secret_managers::secret_lookup::text_to_bool;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::{Section, settings::JsonValue};

const ENABLE_PREVIEW_FEATURES_PATH: &str = "litellm_settings.enable_preview_features";
pub const EXPERIMENTAL_HANDLER_ENV: &str = "EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER";
pub const EMBEDDING_FORMAT_ENV: &str = "LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Global {
    #[serde(default)]
    enable_preview_features: JsonValue,
}

// two bytes on purpose: a pointer would spend Sections' spare bytes under clippy's large_futures
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub experimental_handler: bool,
    pub enable_preview_features: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            // get_secret_bool defaults to unset, so any text but true or false leaves the handler off
            experimental_handler: lookup
                .lookup(EXPERIMENTAL_HANDLER_ENV)
                .as_deref()
                .and_then(text_to_bool)
                .unwrap_or(false),
            enable_preview_features: preview(
                self.litellm_settings.enable_preview_features,
                lookup,
            )?,
        })
    }
}

// LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT has no YAML owner, and a String cannot sit in Sections
pub fn embedding_format(lookup: &dyn EnvironmentLookup) -> Option<String> {
    let format = lookup.lookup(EMBEDDING_FORMAT_ENV);
    format.filter(|value| !value.is_empty())
}

// litellm_settings.enable_preview_features is a module-level flag: litellm_settings only, no environment
fn preview(value: JsonValue, lookup: &dyn EnvironmentLookup) -> Result<bool, ConfigError> {
    match value {
        JsonValue::Null => Ok(false),
        JsonValue::Bool(flag) => Ok(flag),
        JsonValue::String(text) => {
            let resolved = resolve_string(text, ENABLE_PREVIEW_FEATURES_PATH, lookup)?;
            text_to_bool(&resolved).ok_or_else(invalid_preview_value)
        }
        JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(invalid_preview_value())
        }
    }
}

// A value of the wrong type is a field error naming the setting, not a silent false
fn invalid_preview_value() -> ConfigError {
    ConfigError::InvalidScalar {
        path: ENABLE_PREVIEW_FEATURES_PATH.to_owned(),
        value: false,
    }
}
