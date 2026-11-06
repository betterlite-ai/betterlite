use betterlite_config::sections::budget_reset::BudgetResetSettings;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::duration::error::DurationError;
use crate::duration::reset::{Reset, next_standardized_reset_time};
use crate::duration::seconds::duration_in_seconds;

mod tests;

const INVALID_BUDGET_DURATION: &str =
    "invalid budget_duration: use a format like '1h', '24h', '7d', or '30d'";

#[derive(Debug, Clone, Copy)]
pub struct ResetClocks<'zone> {
    pub reset_now: DateTime<Utc>,
    pub calendar_now: DateTime<Utc>,
    pub calendar_timezone: Option<&'zone str>,
}

pub fn compute_budget_reset_at(
    duration: &str,
    settings: &BudgetResetSettings,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<Reset, DurationError> {
    let BudgetResetSettings {
        timezone,
        reset_time_of_day,
    } = settings;
    next_standardized_reset_time(
        duration,
        now,
        Some(timezone.as_str()),
        *reset_time_of_day,
        database,
    )
}

pub fn is_persistable_budget_duration(
    duration: &str,
    settings: &BudgetResetSettings,
    clocks: ResetClocks<'_>,
    database: &TimeZoneDatabase,
) -> Result<bool, DurationError> {
    // every DurationError mirrors one of the source's ValueError or OverflowError paths
    let Ok(budget) = duration_in_seconds(
        duration,
        clocks.calendar_now,
        clocks.calendar_timezone,
        database,
    ) else {
        return Ok(false);
    };
    if budget.seconds <= 0 {
        return Ok(false);
    }
    match compute_budget_reset_at(duration, settings, clocks.reset_now, database) {
        Ok(_reset) => Ok(true),
        Err(_error) => Ok(false),
    }
}

pub fn budget_duration_error(
    duration: Option<&str>,
    settings: &BudgetResetSettings,
    clocks: ResetClocks<'_>,
    database: &TimeZoneDatabase,
) -> Result<Option<String>, DurationError> {
    let Some(budget_duration) = duration else {
        return Ok(None);
    };
    if is_persistable_budget_duration(budget_duration, settings, clocks, database)? {
        return Ok(None);
    }
    Ok(Some(INVALID_BUDGET_DURATION.to_owned()))
}
