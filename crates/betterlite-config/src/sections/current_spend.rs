use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

const FAIL_CLOSED_PATH: &str = "general_settings.fail_closed_budget_enforcement";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    fail_closed_budget_enforcement: Option<JsonValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    // LiteLLM's `is True`: only an explicit true rejects a budget check whose spend nothing verified
    pub fail_closed_budget_enforcement: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // the YAML reader coerces a quoted "true" into a bool, so the declared bool type is checked here
        let fail_closed_budget_enforcement =
            match self.general_settings.fail_closed_budget_enforcement {
                None | Some(JsonValue::Null) => false,
                Some(JsonValue::Bool(flag)) => flag,
                Some(
                    JsonValue::String(_)
                    | JsonValue::Number(_)
                    | JsonValue::Array(_)
                    | JsonValue::Object(_),
                ) => {
                    return Err(ConfigError::InvalidScalar {
                        path: FAIL_CLOSED_PATH.to_owned(),
                        value: false,
                    });
                }
            };
        Ok(Settings {
            fail_closed_budget_enforcement,
        })
    }
}

mod tests;
