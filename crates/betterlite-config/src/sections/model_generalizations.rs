use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const RULES_PATH: &str = "fallback_generalizations.rules";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    fallback_generalizations: Option<RawGeneralizations>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneralizations {
    #[serde(default)]
    rules: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub rules: Vec<JsonValue>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(generalizations) = self.fallback_generalizations else {
            return Ok(Settings::default());
        };
        match generalizations.rules {
            None | Some(JsonValue::Null) => Ok(Settings::default()),
            Some(JsonValue::Array(rules)) => Ok(Settings { rules }),
            Some(value) => Err(ConfigError::InvalidScalar {
                path: RULES_PATH.to_owned(),
                value: matches!(value, JsonValue::Bool(true)),
            }),
        }
    }
}
