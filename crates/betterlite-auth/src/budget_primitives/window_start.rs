use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use speedate::{Date, DateTime as SpeedateDateTime, DateTimeConfig, TimeConfig};

use super::window::BudgetWindow;

pub trait DurationParser {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64>;
}

#[must_use]
pub fn budget_window_start(
    window: &BudgetWindow,
    now: DateTime<Utc>,
    durations: &dyn DurationParser,
) -> Option<DateTime<FixedOffset>> {
    let duration = match window.as_ref().get("budget_duration") {
        Some(JsonData::String(spec)) => spec,
        Some(JsonData::Null) | None => return None,
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => {
            return None;
        }
    };
    let elapsed = TimeDelta::try_seconds(durations.seconds(duration, now)?)?;
    coerce_datetime(window.as_ref().get("reset_at")).map_or_else(
        || {
            now.checked_sub_signed(elapsed)
                .map(|start| start.fixed_offset())
        },
        |reset| reset.checked_sub_signed(elapsed),
    )
}

#[must_use]
pub fn coerce_datetime(value: Option<&JsonData>) -> Option<DateTime<FixedOffset>> {
    let Some(JsonData::String(text)) = value else {
        return None;
    };
    parse_datetime(text)
}

fn parse_datetime(text: &str) -> Option<DateTime<FixedOffset>> {
    let normalized = text.replace('Z', "+00:00");
    // speedate would read a bare number as a unix timestamp; fromisoformat rejects one
    if normalized.parse::<f64>().is_ok() {
        return None;
    }
    let config = truncating_config()?;
    if let Ok(parsed) = SpeedateDateTime::parse_str_with_config(&normalized, &config) {
        return fixed_offset(parsed);
    }
    let date = Date::parse_str(&normalized).ok()?;
    let midnight = format!("{date}T00:00:00");
    fixed_offset(SpeedateDateTime::parse_str_with_config(&midnight, &config).ok()?)
}

fn truncating_config() -> Option<DateTimeConfig> {
    let overflow = "truncate".parse().ok()?;
    let time_config = TimeConfig::builder()
        .microseconds_precision_overflow_behavior(overflow)
        .build();
    Some(DateTimeConfig::builder().time_config(time_config).build())
}

fn fixed_offset(value: SpeedateDateTime) -> Option<DateTime<FixedOffset>> {
    let date = NaiveDate::from_ymd_opt(
        i32::from(value.date.year),
        u32::from(value.date.month),
        u32::from(value.date.day),
    )?;
    let time = NaiveTime::from_hms_micro_opt(
        u32::from(value.time.hour),
        u32::from(value.time.minute),
        u32::from(value.time.second),
        value.time.microsecond,
    )?;
    let offset = FixedOffset::east_opt(value.time.tz_offset.unwrap_or(0_i32))?;
    NaiveDateTime::new(date, time)
        .and_local_timezone(offset)
        .single()
}
