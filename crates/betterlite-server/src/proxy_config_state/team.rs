use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::structured::SensitiveDataMasker;

use super::error::ConfigStateError;
use super::state::ProxyConfigState;

const ENVIRONMENT_PREFIX: &str = "os.environ/";
const LITELLM_SETTINGS: &str = "litellm_settings";
const DEFAULT_TEAM_SETTINGS: &str = "default_team_settings";
const TEAM_ID: &str = "team_id";

// load_team_config: the completion() params the config file gives this team
pub fn team_config(
    state: &ProxyConfigState,
    team_id: &str,
    environment: &dyn EnvironmentLookup,
) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
    let Some(teams) = default_team_settings(state)? else {
        return Ok(BTreeMap::new());
    };
    for entry in teams {
        let JsonValue::Object(fields) = entry else {
            return Err(missing_team_id(entry));
        };
        let Some(entry_team_id) = fields.get(TEAM_ID) else {
            return Err(missing_team_id(entry));
        };
        if matches!(entry_team_id, JsonValue::String(value) if value == team_id) {
            return Ok(resolved_team_config(entry, environment));
        }
    }
    Ok(BTreeMap::new())
}

fn default_team_settings(
    state: &ProxyConfigState,
) -> Result<Option<&[JsonValue]>, ConfigStateError> {
    match state.snapshot.get(LITELLM_SETTINGS) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(settings)) => match settings.get(DEFAULT_TEAM_SETTINGS) {
            None | Some(JsonValue::Null) => Ok(None),
            Some(JsonValue::Array(teams)) => Ok(Some(teams.as_slice())),
            Some(
                JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Object(_),
            ) => Err(not_a_list()),
        },
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => Err(ConfigStateError::InvalidSetting {
            field: LITELLM_SETTINGS,
            expected: "an object",
        }),
    }
}

const fn not_a_list() -> ConfigStateError {
    ConfigStateError::InvalidSetting {
        field: "litellm_settings.default_team_settings",
        expected: "a list",
    }
}

// _get_team_config raises for an entry without team_id and masks the entry, which may carry keys
fn missing_team_id(entry: &JsonValue) -> ConfigStateError {
    ConfigStateError::MissingTeamId {
        team: SensitiveDataMasker::default().mask(entry),
    }
}

// the matched entry with its shallow os.environ/ strings read, as _get_team_config leaves it
fn resolved_team_config(
    entry: &JsonValue,
    environment: &dyn EnvironmentLookup,
) -> BTreeMap<String, JsonValue> {
    let JsonValue::Object(fields) = entry else {
        return BTreeMap::new();
    };
    fields
        .iter()
        .map(|(key, value)| (key.clone(), resolved_environment_value(value, environment)))
        .collect()
}

fn resolved_environment_value(value: &JsonValue, environment: &dyn EnvironmentLookup) -> JsonValue {
    let JsonValue::String(text) = value else {
        return value.clone();
    };
    if !text.starts_with(ENVIRONMENT_PREFIX) {
        return value.clone();
    }
    let name = text.replace(ENVIRONMENT_PREFIX, "");
    environment
        .lookup(&name)
        .map_or(JsonValue::Null, |secret| secret_value(&secret))
}

// get_secret reads os.environ and str_to_bool turns only true and false into a bool
fn secret_value(secret: &str) -> JsonValue {
    let trimmed = secret.trim();
    // ast-grep-ignore: no-python-habits - LiteLLM's str_to_bool takes only true and false, and betterlite_env::flag also takes 1, yes and on
    if trimmed.eq_ignore_ascii_case("true") {
        return JsonValue::Bool(true);
    }
    // ast-grep-ignore: no-python-habits - the same str_to_bool line: every other value stays the raw string the variable holds
    if trimmed.eq_ignore_ascii_case("false") {
        return JsonValue::Bool(false);
    }
    JsonValue::String(secret.to_owned())
}
