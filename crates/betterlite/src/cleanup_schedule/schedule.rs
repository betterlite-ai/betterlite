use betterlite_config::sections::settings::JsonValue;

use super::ports::GeneralSettingsView;
use super::{
    AUTOROUTER_RETENTION_KEY, CLEANUP_CRON_KEY, HEALTH_RETENTION_KEY, RETENTION_INTERVAL_KEY,
    RETENTION_PERIOD_KEY, TAG_SPEND_RETENTION_KEY,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanupSchedule {
    pub spend_logs_retention: Option<JsonValue>,
    pub autorouter_sessions_retention: Option<JsonValue>,
    pub health_checks_retention: Option<JsonValue>,
    pub tag_spend_retention: Option<JsonValue>,
    pub cleanup_cron: Option<JsonValue>,
    pub cleanup_interval: Option<JsonValue>,
}

#[must_use]
pub fn cleanup_schedule_of(settings: &dyn GeneralSettingsView) -> CleanupSchedule {
    CleanupSchedule {
        spend_logs_retention: read_key(settings, RETENTION_PERIOD_KEY),
        autorouter_sessions_retention: read_key(settings, AUTOROUTER_RETENTION_KEY),
        health_checks_retention: read_key(settings, HEALTH_RETENTION_KEY),
        tag_spend_retention: read_key(settings, TAG_SPEND_RETENTION_KEY),
        cleanup_cron: read_key(settings, CLEANUP_CRON_KEY),
        cleanup_interval: read_key(settings, RETENTION_INTERVAL_KEY),
    }
}

// a config file that sets a key to null turns the feature off just as an absent key does
fn read_key(settings: &dyn GeneralSettingsView, key: &str) -> Option<JsonValue> {
    match settings.get(key) {
        None | Some(JsonValue::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

#[must_use]
pub const fn wants_cleanup_job(schedule: &CleanupSchedule) -> bool {
    schedule.spend_logs_retention.is_some()
        || schedule.autorouter_sessions_retention.is_some()
        || schedule.health_checks_retention.is_some()
        || schedule.tag_spend_retention.is_some()
}

// Python truthiness decides whether a cron value is present, so "0", false and [] fall through to the interval
#[must_use]
pub fn is_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_u64().map_or_else(
            || {
                number.as_i64().map_or_else(
                    || {
                        number
                            .as_f64()
                            .is_some_and(|value| value.abs().to_bits() != 0_u64)
                    },
                    |value| value != 0,
                )
            },
            |value| value != 0,
        ),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}

#[must_use]
pub fn value_text(value: &JsonValue) -> String {
    if let JsonValue::String(text) = value {
        return text.clone();
    }
    value.to_string()
}
