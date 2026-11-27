use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

const ENFORCE_FALLBACK_PATH: &str = "general_settings.enforce_fallback_model_access";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "enforce_fallback_model_access")]
    enforce_fallback_model_access: Option<JsonValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub enforce_fallback_model_access: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // the YAML reader coerces a quoted "true" into a bool, so the declared bool type is checked here
        let enforce_fallback_model_access =
            match self.general_settings.enforce_fallback_model_access {
                None | Some(JsonValue::Null) => false,
                Some(JsonValue::Bool(flag)) => flag,
                Some(
                    JsonValue::String(_)
                    | JsonValue::Number(_)
                    | JsonValue::Array(_)
                    | JsonValue::Object(_),
                ) => {
                    return Err(ConfigError::InvalidScalar {
                        path: ENFORCE_FALLBACK_PATH.to_owned(),
                        value: false,
                    });
                }
            };
        Ok(Settings {
            enforce_fallback_model_access,
        })
    }
}

mod tests;
