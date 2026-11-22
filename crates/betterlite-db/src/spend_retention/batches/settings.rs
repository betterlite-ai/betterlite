use std::fmt::Display;
use std::time::Duration;

use num_traits::ToPrimitive;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

pub const BATCH_SIZE_KEY: &str = "maximum_spend_logs_cleanup_batch_size";
pub const MAX_BATCHES_KEY: &str = "maximum_spend_logs_cleanup_max_batches";
pub const RUN_BUDGET_KEY: &str = "maximum_spend_logs_cleanup_run_budget";
pub const BATCH_TIMEOUT_KEY: &str = "maximum_spend_logs_cleanup_batch_timeout";

// general_settings is a plain YAML mapping in LiteLLM, so each override keeps the scalar the operator wrote
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Flag(bool),
    Number(Number),
    Text(String),
    Nested(IgnoredAny),
}

// IgnoredAny always compares equal, so the derived equality is reflexive
impl Eq for SettingValue {}

// B199's duration_in_seconds lives in betterlite-core, which depends on this crate, so the caller supplies it
pub trait DurationParser {
    type Failure: Display;

    fn duration_seconds(&self, spec: &str) -> Result<i64, Self::Failure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupBounds {
    pub batch_size: i64,
    pub max_batches: i64,
    pub run_budget: Duration,
    pub batch_timeout: Duration,
    pub max_consecutive_failures: i64,
    pub failure_backoff: Duration,
    pub remaining_count_cap: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CleanupGeneralSettings {
    #[serde(rename = "maximum_spend_logs_cleanup_batch_size")]
    pub batch_size: Option<SettingValue>,
    #[serde(rename = "maximum_spend_logs_cleanup_max_batches")]
    pub max_batches: Option<SettingValue>,
    #[serde(rename = "maximum_spend_logs_cleanup_run_budget")]
    pub run_budget: Option<SettingValue>,
    #[serde(rename = "maximum_spend_logs_cleanup_batch_timeout")]
    pub batch_timeout: Option<SettingValue>,
    #[serde(rename = "maximum_spend_logs_retention_period")]
    pub spend_logs_retention: Option<SettingValue>,
    #[serde(rename = "maximum_autorouter_session_retention_period")]
    pub session_retention: Option<SettingValue>,
    #[serde(rename = "maximum_health_check_retention_period")]
    pub health_check_retention: Option<SettingValue>,
    #[serde(rename = "maximum_daily_tag_spend_retention_period")]
    pub tag_spend_retention: Option<SettingValue>,
    #[serde(rename = "use_spend_logs_partitioning")]
    pub use_spend_logs_partitioning: Option<SettingValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    SpendLogs,
    AutorouterSessions,
    HealthChecks,
    DailyTagSpend,
}

impl Retention {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::SpendLogs => "maximum_spend_logs_retention_period",
            Self::AutorouterSessions => "maximum_autorouter_session_retention_period",
            Self::HealthChecks => "maximum_health_check_retention_period",
            Self::DailyTagSpend => "maximum_daily_tag_spend_retention_period",
        }
    }
}

// the environment-derived defaults and the live general_settings view one run reads its bounds from
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupSettings {
    pub general: CleanupGeneralSettings,
    pub defaults: CleanupBounds,
}

impl CleanupSettings {
    // _refresh_bounds: every override falls back to its default, so clearing a key restores it
    #[must_use]
    pub fn bounds<Parser>(&self, parser: &Parser) -> CleanupBounds
    where
        Parser: DurationParser,
    {
        let general = &self.general;
        let defaults = self.defaults;
        CleanupBounds {
            batch_size: positive_setting(
                BATCH_SIZE_KEY,
                general.batch_size.as_ref(),
                defaults.batch_size,
            ),
            max_batches: positive_setting(
                MAX_BATCHES_KEY,
                general.max_batches.as_ref(),
                defaults.max_batches,
            ),
            run_budget: duration_setting(
                RUN_BUDGET_KEY,
                general.run_budget.as_ref(),
                defaults.run_budget,
                parser,
            ),
            batch_timeout: duration_setting(
                BATCH_TIMEOUT_KEY,
                general.batch_timeout.as_ref(),
                defaults.batch_timeout,
                parser,
            ),
            max_consecutive_failures: defaults.max_consecutive_failures,
            failure_backoff: defaults.failure_backoff,
            remaining_count_cap: defaults.remaining_count_cap,
        }
    }

    // _retention_seconds_for: None disables the group; zero and other parsed values are kept as given
    #[must_use]
    pub fn retention_seconds<Parser>(&self, retention: Retention, parser: &Parser) -> Option<i64>
    where
        Parser: DurationParser,
    {
        let configured = match retention {
            Retention::SpendLogs => self.general.spend_logs_retention.as_ref(),
            Retention::AutorouterSessions => self.general.session_retention.as_ref(),
            Retention::HealthChecks => self.general.health_check_retention.as_ref(),
            Retention::DailyTagSpend => self.general.tag_spend_retention.as_ref(),
        };
        let key = retention.key();
        tracing::info!("Checking {key}: {configured:?}");
        let spec = retention_spec(key, configured?)?;
        match parser.duration_seconds(&spec) {
            Ok(seconds) => {
                tracing::info!("{key} set to {seconds} seconds");
                Some(seconds)
            }
            Err(error) => {
                tracing::warn!("Invalid {key} value: {spec}, error: {error}");
                None
            }
        }
    }
}

// int(raw): a bool counts as 0 or 1 and a float truncates toward zero
fn override_integer(value: &SettingValue) -> Option<i64> {
    match value {
        SettingValue::Flag(flag) => Some(i64::from(*flag)),
        SettingValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().as_ref().and_then(ToPrimitive::to_i64)),
        SettingValue::Text(text) => text.trim().parse::<i64>().ok(),
        SettingValue::Nested(_) => None,
    }
}

fn positive_setting(key: &str, configured: Option<&SettingValue>, default: i64) -> i64 {
    let Some(value) = configured else {
        return default;
    };
    let Some(parsed) = override_integer(value) else {
        tracing::warn!("Invalid {key} value: {value:?}, using default {default}");
        return default;
    };
    if parsed <= 0 {
        tracing::warn!("{key} must be positive, got {parsed}, using default {default}");
        return default;
    }
    parsed
}

// duration_in_seconds(str(raw)) only accepts text, since str() of any other value has no unit
fn duration_setting<Parser>(
    key: &str,
    configured: Option<&SettingValue>,
    default: Duration,
    parser: &Parser,
) -> Duration
where
    Parser: DurationParser,
{
    let Some(value) = configured else {
        return default;
    };
    let SettingValue::Text(spec) = value else {
        tracing::warn!("Invalid {key} value: {value:?}, using default {default:?}");
        return default;
    };
    let seconds = match parser.duration_seconds(spec) {
        Ok(seconds) => seconds,
        Err(error) => {
            tracing::warn!("Invalid {key} value: {spec} ({error}), using default {default:?}");
            return default;
        }
    };
    match u64::try_from(seconds) {
        Ok(positive) if positive > 0 => Duration::from_secs(positive),
        Ok(_) | Err(_) => {
            tracing::warn!(
                "{key} must be a positive duration, got {spec}, using default {default:?}"
            );
            default
        }
    }
}

// an integer means days; LiteLLM raises past its ValueError handler on a float, list or mapping and aborts the run
fn retention_spec(key: &str, value: &SettingValue) -> Option<String> {
    match value {
        SettingValue::Text(text) => Some(text.clone()),
        SettingValue::Number(number) => {
            let Some(days) = number.as_i64() else {
                tracing::warn!("Invalid {key} value: {number}, expected a duration string");
                return None;
            };
            tracing::warn!(
                "{key} is an integer ({days}); treating as days. Use a string like '3d' to be explicit."
            );
            Some(format!("{days}d"))
        }
        SettingValue::Flag(_) | SettingValue::Nested(_) => {
            tracing::warn!("Invalid {key} value: {value:?}");
            None
        }
    }
}
