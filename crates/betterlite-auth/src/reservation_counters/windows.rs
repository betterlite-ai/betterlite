use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::Payload;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use chrono::{DateTime, FixedOffset, Utc};
use rust_decimal::Decimal;

use crate::budget_counter::coercion::FloatInput as _;
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::budget_primitives::window::{BudgetWindow, coerce_window};
use crate::budget_primitives::window_start::{DurationParser, budget_window_start};

pub(crate) struct WindowEntity {
    pub(crate) prefix: String,
    pub(crate) entity_type: CounterEntityType,
    pub(crate) entity_id: String,
}

pub(crate) fn window_counters(
    entity: &WindowEntity,
    windows: &[JsonData],
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Vec<BudgetCounter> {
    let mut counters = Vec::with_capacity(windows.len());
    for value in windows {
        let window = coerce_window(value);
        let Some(duration) = window_duration(&window) else {
            continue;
        };
        let Some(max_budget) = window_amount(window.as_ref().get("max_budget")) else {
            continue;
        };
        if max_budget <= Decimal::ZERO {
            continue;
        }
        let Some(start) = budget_window_start(&window, now, durations) else {
            // LiteLLM prints the entity id here; a key's id is the key token, so only the type and duration go out
            tracing::warn!(
                entity_type = entity.entity_type.wire_value(),
                budget_duration = %duration,
                "Skipping budget window with invalid duration"
            );
            continue;
        };
        let Some(start) = system_time(start) else {
            tracing::warn!(
                entity_type = entity.entity_type.wire_value(),
                budget_duration = %duration,
                "Skipping budget window outside the supported clock range"
            );
            continue;
        };
        counters.push(BudgetCounter {
            counter_key: format!("{}:window:{duration}", entity.prefix),
            max_budget,
            fallback_spend: Decimal::ZERO,
            entity_type: entity.entity_type.clone(),
            entity_id: format!("{}:{duration}", entity.entity_id),
            source_cache_key: None,
            spend_log_entity_id: Some(entity.entity_id.clone()),
            window_duration: Some(duration),
            window_start: Some(start),
        });
    }
    counters
}

fn window_duration(window: &BudgetWindow) -> Option<String> {
    match window.as_ref().get("budget_duration") {
        Some(JsonData::String(spec)) if !spec.is_empty() => Some(spec.clone()),
        Some(JsonData::String(_) | JsonData::Null) | None => None,
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => {
            tracing::warn!("Skipping budget window with a budget_duration that is not a string");
            None
        }
    }
}

fn window_amount(value: Option<&JsonData>) -> Option<Decimal> {
    match value? {
        JsonData::Number(number) => number.to_string().decimal_value(),
        JsonData::String(text) => text.as_str().decimal_value(),
        JsonData::Bool(flag) => flag.decimal_value(),
        JsonData::Null | JsonData::Array(_) | JsonData::Object(_) => None,
    }
}

pub(crate) fn payload_windows(limits: Option<&[Payload]>) -> Vec<JsonData> {
    limits
        .unwrap_or_default()
        .iter()
        .map(|limit| JsonData::Object(limit.0.clone()))
        .collect()
}

pub(crate) fn entry_window(entry: &BudgetLimitEntry) -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert(
        "budget_duration".to_owned(),
        JsonData::String(entry.budget_duration.clone()),
    );
    fields.insert(
        "max_budget".to_owned(),
        JsonData::String(entry.max_budget.to_string()),
    );
    if let Some(reset_at) = entry.reset_at {
        fields.insert(
            "reset_at".to_owned(),
            JsonData::String(timestamp_text(reset_at)),
        );
    }
    JsonData::Object(fields)
}

fn timestamp_text(timestamp: Timestamp) -> String {
    match timestamp {
        Timestamp::Aware(value) => value.to_rfc3339(),
        Timestamp::Naive(value) => value.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
    }
}

// chrono's From<DateTime<FixedOffset>> for SystemTime adds without checking, and a stored reset time can sit past the clock
fn system_time(moment: DateTime<FixedOffset>) -> Option<SystemTime> {
    let seconds = moment.timestamp();
    let nanos = moment.timestamp_subsec_nanos();
    let base = if seconds.is_negative() {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))?
    } else {
        UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(seconds).ok()?))?
    };
    base.checked_add(Duration::from_nanos(u64::from(nanos)))
}
