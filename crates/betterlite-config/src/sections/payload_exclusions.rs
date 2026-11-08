#[path = "payload_exclusions/tests.rs"]
mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use serde::Deserialize;

const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER";
const DEFAULT_MAX_DEPTH: i64 = 10;
const EXCLUDED_FIELDS_PATH: &str = "litellm_settings.standard_logging_payload_excluded_fields";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    #[serde(rename = "standard_logging_payload_excluded_fields")]
    excluded_fields: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub excluded_fields: Vec<String>,
    pub max_redaction_depth: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            excluded_fields: Vec::new(),
            max_redaction_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let invalid = |value: &JsonValue| ConfigError::InvalidScalar {
            path: EXCLUDED_FIELDS_PATH.to_owned(),
            value: matches!(value, JsonValue::Bool(true)),
        };
        let excluded_fields = match self.litellm_settings.excluded_fields {
            None | Some(JsonValue::Null) => Vec::new(),
            Some(JsonValue::Array(items)) => {
                let mut fields = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        JsonValue::String(field) => fields.push(field),
                        other @ (JsonValue::Null
                        | JsonValue::Bool(_)
                        | JsonValue::Number(_)
                        | JsonValue::Array(_)
                        | JsonValue::Object(_)) => return Err(invalid(&other)),
                    }
                }
                fields
            }
            Some(value) => return Err(invalid(&value)),
        };
        let max_redaction_depth = match lookup.lookup(MAX_DEPTH_VARIABLE) {
            None => DEFAULT_MAX_DEPTH,
            Some(text) => betterlite_env::number::<i64>(MAX_DEPTH_VARIABLE, &text)?
                .unwrap_or(DEFAULT_MAX_DEPTH),
        };
        Ok(Settings {
            excluded_fields,
            max_redaction_depth,
        })
    }
}
