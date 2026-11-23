mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

const DEFAULT_BUDGET_PATH: &str = "litellm_settings.max_end_user_budget_id";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Global {
    #[serde(default)]
    max_end_user_budget_id: JsonValue,
}

// litellm.max_end_user_budget_id: the budget row a key's end_user_budget_id can override
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub max_end_user_budget_id: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            max_end_user_budget_id: default_budget(
                self.litellm_settings.max_end_user_budget_id,
                lookup,
            )?,
        })
    }
}

fn default_budget(
    value: JsonValue,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<String>, ConfigError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::String(text) => {
            let from_environment = text.starts_with("os.environ/");
            let resolved = resolve_string(text, DEFAULT_BUDGET_PATH, lookup)?;
            // an empty environment variable counts as unset; a literal "" is the explicit value
            if from_environment && resolved.is_empty() {
                return Ok(None);
            }
            Ok(Some(resolved))
        }
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ConfigError::InvalidScalar {
                path: DEFAULT_BUDGET_PATH.to_owned(),
                value: false,
            })
        }
    }
}
