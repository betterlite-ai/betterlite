mod cold_storage;
mod monitor;
mod partition_switch;
mod queue;
pub mod retention;
mod tests;
mod writer;
use self::cold_storage::custom_logger;
use self::queue::{logs_url, max_bytes};
use self::writer::{SpendLogSwitch, disable_spend_logs, max_budget};
use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::raw::RawNumber;
use crate::sections::Section;
use crate::sections::settings::JsonValue;
use betterlite_db::spend_flush::monitor::SpendMonitorSettings;
use betterlite_db::spend_partitions::calendar::PartitionInterval;
use betterlite_db::spend_retention::batches::settings::{CleanupGeneralSettings, CleanupSettings};
use betterlite_secret_managers::secret_lookup::text_to_bool;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeMap;
pub const ENV_STORE_PROMPTS: &str = "STORE_PROMPTS_IN_SPEND_LOGS";
pub const ENV_MAX_PROMPT_LENGTH: &str = "MAX_STRING_LENGTH_PROMPT_IN_DB";
pub const DEFAULT_MAX_PROMPT_LENGTH: i64 = 2048;
pub const ENV_DEQUEUE_COUNT: &str = "MAX_REDIS_BUFFER_DEQUEUE_COUNT";
pub const DEFAULT_DEQUEUE_COUNT: i64 = 100;
pub const ENV_PARTITION_INTERVAL: &str = "SPEND_LOG_PARTITION_INTERVAL";
pub const ENV_PRECREATE_AHEAD: &str = "SPEND_LOG_PARTITION_PRECREATE_AHEAD";
pub const DEFAULT_PRECREATE_AHEAD: i64 = 7;
const MISSING_SESSION_ID_PATH: &str = "general_settings.missing_session_id";
const USE_REDIS_BUFFER_PATH: &str = "general_settings.use_redis_transaction_buffer";
const DISABLE_SPEND_LOGS_PATH: &str = "general_settings.disable_spend_logs";
const MAX_BUDGET_PATH: &str = "litellm_settings.max_budget";
const DISABLE_END_USER_PATH: &str = "litellm_settings.disable_end_user_cost_tracking";
const ENABLE_PROMETHEUS_PATH: &str =
    "litellm_settings.enable_end_user_cost_tracking_prometheus_only";
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    pub general_settings: GeneralSettings,
    pub litellm_settings: LiteLlmSettings,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LiteLlmSettings {
    max_budget: Option<JsonValue>,
    disable_end_user_cost_tracking: JsonValue,
    #[serde(rename = "enable_end_user_cost_tracking_prometheus_only")]
    enable_prometheus_only: JsonValue,
    cold_storage_custom_logger: JsonValue,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    store_prompts_in_spend_logs: JsonValue,
    missing_session_id: JsonValue,
    disable_spend_updates: JsonValue,
    // absent keeps the proxy's False, an explicit null stays "not exactly false"
    disable_spend_logs: SpendLogSwitch,
    use_redis_transaction_buffer: JsonValue,
    #[serde(flatten)]
    retention: CleanupGeneralSettings,
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
    pub disable_spend_logs: Option<bool>,
    pub max_budget: Decimal,
    pub disable_end_user_cost_tracking: Option<bool>,
    pub enable_prometheus_only: Option<bool>,
    pub use_redis_transaction_buffer: bool,
    pub max_redis_buffer_dequeue_count: i64,
    pub partition_interval: PartitionInterval,
    pub precreate_ahead: i64,
    pub spend_log_queue_max_bytes: usize,
    pub spend_logs_url: Option<String>,
    // the spend log queue monitor's threshold and base poll interval, read from the environment once with the rest of the section
    pub spend_log_monitor: SpendMonitorSettings,
    pub retention: CleanupSettings,
    // the callback name ColdStorageHandler reads stored request bodies through; None reads none
    pub cold_storage_custom_logger: Option<String>,
}
impl Section for Raw {
    type Settings = Settings;
    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let litellm = self.litellm_settings;
        let (use_redis, dequeue_count, interval, ahead) = runtime_reads(&general, lookup)?;
        Ok(Settings {
            store_prompts_in_spend_logs: store_prompts(
                &general.store_prompts_in_spend_logs,
                lookup.lookup(ENV_STORE_PROMPTS).as_deref(),
            ),
            max_string_length_prompt_in_db: initial_environment_integer(
                lookup,
                ENV_MAX_PROMPT_LENGTH,
                DEFAULT_MAX_PROMPT_LENGTH,
            )?,
            missing_session_id: missing_session_id(&general.missing_session_id)?,
            spend_updates: spend_updates(&general.disable_spend_updates),
            disable_spend_logs: disable_spend_logs(&general.disable_spend_logs)?,
            max_budget: max_budget(litellm.max_budget, lookup)?,
            disable_end_user_cost_tracking: optional_flag(
                &litellm.disable_end_user_cost_tracking,
                DISABLE_END_USER_PATH,
            )?,
            enable_prometheus_only: optional_flag(
                &litellm.enable_prometheus_only,
                ENABLE_PROMETHEUS_PATH,
            )?,
            use_redis_transaction_buffer: use_redis,
            max_redis_buffer_dequeue_count: dequeue_count,
            partition_interval: interval,
            precreate_ahead: ahead,
            spend_log_queue_max_bytes: max_bytes(lookup)?,
            spend_logs_url: logs_url(lookup),
            spend_log_monitor: monitor::monitor_settings(lookup)?,
            retention: retention::cleanup_settings(general.retention, lookup)?,
            cold_storage_custom_logger: custom_logger(&litellm.cold_storage_custom_logger)?,
        })
    }
}
fn runtime_reads<L>(
    general: &GeneralSettings,
    lookup: &L,
) -> Result<(bool, i64, PartitionInterval, i64), ConfigError>
where
    L: EnvironmentLookup,
{
    let use_redis = redis_transaction_buffer(&general.use_redis_transaction_buffer)?;
    let dequeue_count =
        initial_environment_integer(lookup, ENV_DEQUEUE_COUNT, DEFAULT_DEQUEUE_COUNT)?;
    let interval = partition_interval(lookup.lookup(ENV_PARTITION_INTERVAL).as_deref());
    let ahead = initial_environment_integer(lookup, ENV_PRECREATE_AHEAD, DEFAULT_PRECREATE_AHEAD)?;
    Ok((use_redis, dequeue_count, interval, ahead))
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
    pub litellm_settings: LiteLlmSettings,
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
        litellm_settings: input.litellm_settings,
    };
    Ok(Section::convert(raw, &input.environment)?)
}
fn store_prompts(configured: &JsonValue, environment: Option<&str>) -> bool {
    let enabled = matches!(configured, JsonValue::Bool(true))
        || matches!(configured, JsonValue::String(text) if text.to_lowercase() == "true");
    enabled || environment.and_then(text_to_bool).unwrap_or(false)
}
// an empty value counts as unset, and the load fails on text that names this variable
fn initial_environment_integer<L>(lookup: &L, name: &str, fallback: i64) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    lookup.lookup(name).map_or(Ok(fallback), |text| {
        let number = betterlite_env::number::<i64>(name, &text)
            .map_err(|source| ConfigError::Environment { source })?;
        Ok(number.unwrap_or(fallback))
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
// litellm reads the setting as bool | str | None; a string goes through str_to_bool, so only true and false are on.
fn redis_transaction_buffer(configured: &JsonValue) -> Result<bool, ConfigError> {
    match configured {
        JsonValue::Null => Ok(false),
        JsonValue::Bool(flag) => Ok(*flag),
        JsonValue::String(text) => Ok(text_to_bool(text).unwrap_or(false)),
        JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(ConfigError::InvalidScalar {
                path: USE_REDIS_BUFFER_PATH.to_owned(),
                value: false,
            })
        }
    }
}
// Zero and negative counts reach Redis unchanged, so the server's own LPOP rules apply rather than a floor of ours.
#[must_use]
pub fn runtime_dequeue_count(initial: i64, snapshot: Option<&str>) -> i64 {
    let Some(text) = snapshot else {
        return initial;
    };
    match betterlite_env::number::<i64>(ENV_DEQUEUE_COUNT, text) {
        Ok(Some(count)) => count,
        Ok(None) | Err(_) => initial,
    }
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
// litellm sets both end-user switches as Optional[bool] = None, so only a boolean or a null loads.
fn optional_flag(configured: &JsonValue, path: &str) -> Result<Option<bool>, ConfigError> {
    match configured {
        JsonValue::Null => Ok(None),
        JsonValue::Bool(flag) => Ok(Some(*flag)),
        JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}

// an unrecognized SPEND_LOG_PARTITION_INTERVAL warns and uses day, as SpendLogsPartitionManager.__init__ does
fn partition_interval(value: Option<&str>) -> PartitionInterval {
    let Some(text) = value.filter(|text| !text.is_empty()) else {
        return PartitionInterval::Day;
    };
    let Some(interval) = PartitionInterval::from_name(text) else {
        tracing::warn!(
            "Invalid SPEND_LOG_PARTITION_INTERVAL {text:?}, falling back to 'day'; supported values are day, week and month"
        );
        return PartitionInterval::Day;
    };
    interval
}

fn budget_number(configured: JsonValue) -> Result<RawNumber, ConfigError> {
    let number = match configured {
        JsonValue::Number(number) => number.as_f64().map(RawNumber::Float),
        JsonValue::String(text) => Some(RawNumber::Text(text)),
        JsonValue::Bool(flag) => Some(RawNumber::Boolean(flag)),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    };
    number.ok_or_else(|| ConfigError::InvalidScalar {
        path: MAX_BUDGET_PATH.to_owned(),
        value: false,
    })
}
