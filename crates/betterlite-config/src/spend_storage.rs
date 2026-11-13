mod tests;
use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use betterlite_secret_managers::secret_lookup::text_to_bool;
use serde::Deserialize;
use std::collections::BTreeMap;
pub const ENV_STORE_PROMPTS: &str = "STORE_PROMPTS_IN_SPEND_LOGS";
pub const ENV_MAX_PROMPT_LENGTH: &str = "MAX_STRING_LENGTH_PROMPT_IN_DB";
pub const DEFAULT_MAX_PROMPT_LENGTH: i64 = 2048;
const MISSING_SESSION_ID_PATH: &str = "general_settings.missing_session_id";
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    pub general_settings: GeneralSettings,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    store_prompts_in_spend_logs: JsonValue,
    missing_session_id: JsonValue,
    disable_spend_updates: JsonValue,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SpendUpdates {
    #[default]
    Allowed,
    Disabled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingSessionId {
    Generate,
    Reject,
    Omit,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub store_prompts_in_spend_logs: bool,
    pub max_string_length_prompt_in_db: i64,
    pub missing_session_id: Option<MissingSessionId>,
    pub spend_updates: SpendUpdates,
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        Ok(Settings {
            store_prompts_in_spend_logs: store_prompts(
                &general.store_prompts_in_spend_logs,
                lookup.lookup(ENV_STORE_PROMPTS).as_deref(),
            ),
            max_string_length_prompt_in_db: initial_max_prompt_length(
                lookup.lookup(ENV_MAX_PROMPT_LENGTH).as_deref(),
            )?,
            missing_session_id: missing_session_id(&general.missing_session_id)?,
            spend_updates: spend_updates(&general.disable_spend_updates),
        })
    }
}
pub fn load_spend_storage_policy<L>(source: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    Section::convert(raw, lookup)
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PolicyInput {
    pub general_settings: GeneralSettings,
    pub environment: BTreeMap<String, String>,
}
#[derive(Debug, thiserror::Error)]
pub enum SpendStorageError {
    #[error("invalid spend storage policy: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
}
pub fn decode_spend_storage_policy(bytes: &[u8]) -> Result<Settings, SpendStorageError> {
    let input: PolicyInput = serde_json::from_slice(bytes)?;
    let raw = Raw {
        general_settings: input.general_settings,
    };
    Ok(Section::convert(raw, &input.environment)?)
}
fn store_prompts(configured: &JsonValue, environment: Option<&str>) -> bool {
    let enabled = matches!(configured, JsonValue::Bool(true))
        || matches!(configured, JsonValue::String(text) if text.to_lowercase() == "true");
    enabled || environment.and_then(text_to_bool).unwrap_or(false)
}
fn initial_max_prompt_length(value: Option<&str>) -> Result<i64, ConfigError> {
    value.map_or(Ok(DEFAULT_MAX_PROMPT_LENGTH), |text| {
        let length = betterlite_env::number::<i64>(ENV_MAX_PROMPT_LENGTH, text)
            .map_err(|source| ConfigError::Environment { source })?;
        Ok(length.unwrap_or(DEFAULT_MAX_PROMPT_LENGTH))
    })
}
#[must_use]
pub fn runtime_max_prompt_length(initial: i64, snapshot: Option<&str>) -> i64 {
    snapshot.map_or(initial, |text| {
        betterlite_env::number::<i64>(ENV_MAX_PROMPT_LENGTH, text)
            .ok()
            .flatten()
            .unwrap_or(initial)
    })
}
fn missing_session_id(value: &JsonValue) -> Result<Option<MissingSessionId>, ConfigError> {
    if value.is_null() {
        return Ok(None);
    }
    match value.as_str() {
        Some("generate") => Ok(Some(MissingSessionId::Generate)),
        Some("reject") => Ok(Some(MissingSessionId::Reject)),
        Some("omit") => Ok(Some(MissingSessionId::Omit)),
        Some(_) | None => Err(ConfigError::InvalidScalar {
            path: MISSING_SESSION_ID_PATH.to_owned(),
            value: false,
        }),
    }
}
const fn spend_updates(value: &JsonValue) -> SpendUpdates {
    if matches!(value, JsonValue::Bool(true)) {
        SpendUpdates::Disabled
    } else {
        SpendUpdates::Allowed
    }
}
