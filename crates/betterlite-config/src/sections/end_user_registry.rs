mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

const VALIDATION_PATH: &str = "litellm_settings.validate_end_user_id_in_db";
const DEFAULT_BUDGET_PATH: &str = "litellm_settings.max_end_user_budget_id";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Global,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Global {
    #[serde(default)]
    validate_end_user_id_in_db: JsonValue,
    #[serde(default)]
    max_end_user_budget_id: JsonValue,
}

// the two module-level flags the restricted end-user registry skip reads in LiteLLM
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub validate_end_user_id_in_db: bool,
    pub max_end_user_budget_id: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            validate_end_user_id_in_db: validation(
                self.litellm_settings.validate_end_user_id_in_db,
                lookup,
            )?,
            max_end_user_budget_id: default_budget(
                self.litellm_settings.max_end_user_budget_id,
                lookup,
            )?,
        })
    }
}

fn validation(value: JsonValue, lookup: &dyn EnvironmentLookup) -> Result<bool, ConfigError> {
    match value {
        JsonValue::Null => Ok(false),
        JsonValue::Bool(flag) => Ok(flag),
        JsonValue::String(text) => {
            let name = text
                .strip_prefix("os.environ/")
                .unwrap_or(VALIDATION_PATH)
                .to_owned();
            let resolved = resolve_string(text, VALIDATION_PATH, lookup)?;
            // the shared parser reads 1, yes and on too, and an empty env var leaves the false default
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
        // a value of the wrong type is a field error naming the setting, not a silent false
        JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ConfigError::InvalidScalar {
                path: VALIDATION_PATH.to_owned(),
                value: false,
            })
        }
    }
}

fn default_budget(
    value: JsonValue,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<String>, ConfigError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::String(text) => {
            let reference = text.starts_with("os.environ/");
            let resolved = resolve_string(text, DEFAULT_BUDGET_PATH, lookup)?;
            // an empty environment variable counts as unset; a literal "" is the explicit value
            Ok((!reference || !resolved.is_empty()).then_some(resolved))
        }
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ConfigError::InvalidScalar {
                path: DEFAULT_BUDGET_PATH.to_owned(),
                value: false,
            })
        }
    }
}
