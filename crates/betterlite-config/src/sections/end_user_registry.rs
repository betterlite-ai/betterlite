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

fn invalid(path: &str) -> ConfigError {
    // a value of the wrong type is a field error naming the setting, not a silent default
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}

// a configured string setting, with the name a parser error reports and whether it came from the environment
struct ResolvedSetting {
    name: String,
    text: String,
    from_environment: bool,
}

fn setting(
    value: JsonValue,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<ResolvedSetting>, ConfigError> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::String(text) => {
            let reference = text.strip_prefix("os.environ/");
            let from_environment = reference.is_some();
            let name = reference.map_or_else(|| path.to_owned(), str::to_owned);
            let resolved = resolve_string(text, path, lookup)?;
            Ok(Some(ResolvedSetting {
                name,
                text: resolved,
                from_environment,
            }))
        }
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(invalid(path))
        }
    }
}

fn validation(value: JsonValue, lookup: &dyn EnvironmentLookup) -> Result<bool, ConfigError> {
    if let JsonValue::Bool(flag) = value {
        return Ok(flag);
    }
    let Some(resolved) = setting(value, VALIDATION_PATH, lookup)? else {
        return Ok(false);
    };
    // the shared parser reads 1, yes and on too, and an empty env var leaves the false default
    Ok(betterlite_env::flag(&resolved.name, &resolved.text)?.unwrap_or(false))
}

fn default_budget(
    value: JsonValue,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<String>, ConfigError> {
    let configured = setting(value, DEFAULT_BUDGET_PATH, lookup)?;
    if let Some(resolved) = configured {
        // an empty environment variable counts as unset; a literal "" is the explicit value
        if resolved.from_environment && resolved.text.is_empty() {
            return Ok(None);
        }
        return Ok(Some(resolved.text));
    }
    Ok(None)
}
