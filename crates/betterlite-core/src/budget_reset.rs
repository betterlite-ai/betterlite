use betterlite_config::sections::budget_reset::{
    BudgetResetSettings, BudgetResetSettingsError, get_budget_reset_settings,
};
use betterlite_config::sections::settings::JsonValue;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::duration::error::DurationError;
use crate::duration::reset::{Reset, next_standardized_reset_time};
use crate::duration::seconds::duration_in_seconds;

mod tests;

const INVALID_BUDGET_DURATION: &str =
    "invalid budget_duration: use a format like '1h', '24h', '7d', or '30d'";

#[derive(Debug, thiserror::Error)]
pub enum BudgetResetError {
    #[error(transparent)]
    Duration(#[from] DurationError),
    #[error(transparent)]
    Settings(#[from] BudgetResetSettingsError),
}

#[derive(Debug, Clone, Copy)]
pub struct RawBudgetResetSettings<'raw> {
    pub timezone: &'raw JsonValue,
    pub reset_time_of_day: &'raw JsonValue,
}

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

pub fn get_budget_reset_time(
    duration: &str,
    raw: RawBudgetResetSettings<'_>,
    now: DateTime<Utc>,
    database: &TimeZoneDatabase,
) -> Result<Reset, BudgetResetError> {
    match get_budget_reset_settings(raw.timezone, raw.reset_time_of_day) {
        Ok(settings) => compute_budget_reset_at(duration, &settings, now, database)
            .map_err(BudgetResetError::Duration),
        Err(error) => Err(BudgetResetError::Settings(error)),
    }
}

pub fn is_persistable_budget_duration(
    duration: &str,
    raw: RawBudgetResetSettings<'_>,
    clocks: ResetClocks<'_>,
    database: &TimeZoneDatabase,
) -> Result<bool, BudgetResetError> {
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
    match get_budget_reset_time(duration, raw, clocks.reset_now, database) {
        Ok(_reset) => Ok(true),
        Err(BudgetResetError::Duration(_error)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn budget_duration_error(
    duration: Option<&str>,
    raw: RawBudgetResetSettings<'_>,
    clocks: ResetClocks<'_>,
    database: &TimeZoneDatabase,
) -> Result<Option<String>, BudgetResetError> {
    let Some(budget_duration) = duration else {
        return Ok(None);
    };
    if is_persistable_budget_duration(budget_duration, raw, clocks, database)? {
        return Ok(None);
    }
    Ok(Some(INVALID_BUDGET_DURATION.to_owned()))
}
