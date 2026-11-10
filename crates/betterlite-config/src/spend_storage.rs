mod tests;

use std::collections::BTreeMap;

use serde::Deserialize;

use betterlite_secret_managers::secret_lookup::text_to_bool;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const ENV_STORE_PROMPTS: &str = "STORE_PROMPTS_IN_SPEND_LOGS";
pub const ENV_MAX_PROMPT_LENGTH: &str = "MAX_STRING_LENGTH_PROMPT_IN_DB";
pub const DEFAULT_MAX_PROMPT_LENGTH: i64 = 2048;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub general_settings: GeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
pub struct GeneralSettings {
    #[serde(default)]
    store_prompts_in_spend_logs: JsonValue,
    #[serde(default)]
    missing_session_id: JsonValue,
    #[serde(default)]
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

impl SpendUpdates {
    #[must_use]
    pub const fn is_disabled(self) -> bool {
        match self {
            Self::Allowed => false,
            Self::Disabled => true,
        }
    }
}

impl Settings {
    #[must_use]
    pub const fn omits_missing_session(&self) -> bool {
        match self.missing_session_id {
            Some(MissingSessionId::Omit) => true,
            Some(MissingSessionId::Generate | MissingSessionId::Reject) | None => false,
        }
    }
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
            missing_session_id: missing_session_id(&general.missing_session_id),
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

#[derive(Debug, Deserialize)]
pub struct PolicyInput {
    #[serde(default)]
    pub general_settings: GeneralSettings,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendStorageError {
    #[error("invalid spend storage policy: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Config(#[from] ConfigError),
}

pub fn decode_spend_storage_policy(bytes: &[u8]) -> Result<Settings, SpendStorageError> {
    let input: PolicyInput = serde_json::from_slice(bytes)?;
    let PolicyInput {
        general_settings,
        environment,
    } = input;
    let settings = Section::convert(Raw { general_settings }, &environment)?;
    Ok(settings)
}

fn store_prompts(configured: &JsonValue, environment: Option<&str>) -> bool {
    let enabled = match configured {
        JsonValue::Bool(flag) => *flag,
        JsonValue::String(text) => text.to_lowercase() == "true",
        JsonValue::Null | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            false
        }
    };
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

fn missing_session_id(value: &JsonValue) -> Option<MissingSessionId> {
    match value.as_str()? {
        "generate" => Some(MissingSessionId::Generate),
        "reject" => Some(MissingSessionId::Reject),
        "omit" => Some(MissingSessionId::Omit),
        _ => None,
    }
}

const fn spend_updates(value: &JsonValue) -> SpendUpdates {
    if matches!(value, JsonValue::Bool(true)) {
        SpendUpdates::Disabled
    } else {
        SpendUpdates::Allowed
    }
}
