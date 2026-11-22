use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const MOCK_TESTING_PATH: &str = "general_settings.dangerously_allow_mock_testing_request_params";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawGeneral {
    #[serde(default)]
    #[serde(rename = "dangerously_allow_mock_testing_request_params")]
    allow_mock_testing: JsonValue,
}

// LiteLLM's mock_testing_params_allowed reads this with `is True`, so only a real bool opts in
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub allowed: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let allowed = match self.general_settings.allow_mock_testing {
            JsonValue::Null => false,
            JsonValue::Bool(flag) => flag,
            JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => {
                return Err(ConfigError::InvalidScalar {
                    path: MOCK_TESTING_PATH.to_owned(),
                    value: false,
                });
            }
        };
        Ok(Settings { allowed })
    }
}
