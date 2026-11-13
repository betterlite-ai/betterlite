use chrono::{DateTime, Utc};
use jiff::civil;
use jiff::tz::TimeZoneDatabase;

use super::error::{DurationError, DurationWarning};
use super::spec::{self, DurationUnit};
use super::zone;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetDuration {
    pub seconds: i64,
    pub warnings: Vec<DurationWarning>,
}

pub fn duration_in_seconds(
    spec: &str,
    current_time: DateTime<Utc>,
    timezone: Option<&str>,
    database: &TimeZoneDatabase,
) -> Result<BudgetDuration, DurationError> {
    let parsed = spec::parse_budget_duration(spec)?;
    let mut warnings = parsed.warnings;
    let seconds = match parsed.unit {
        DurationUnit::Second => scaled_seconds(parsed.amount, 1, spec)?,
        DurationUnit::Minute => scaled_seconds(parsed.amount, 60, spec)?,
        DurationUnit::Hour => scaled_seconds(parsed.amount, 3_600, spec)?,
        DurationUnit::Day => scaled_seconds(parsed.amount, 86_400, spec)?,
        DurationUnit::Week => scaled_seconds(parsed.amount, 604_800, spec)?,
        DurationUnit::Month => {
            let resolved = zone::resolve_zone(timezone, database);
            warnings.extend(resolved.warnings);
            let (moment, _fold) = zone::civil_time(current_time, &resolved.zone)?;
            month_seconds(parsed.amount, moment)?
        }
    };
    Ok(BudgetDuration { seconds, warnings })
}

fn scaled_seconds(amount: u64, factor: u64, spec: &str) -> Result<i64, DurationError> {
    let scaled = u128::from(amount)
        .checked_mul(u128::from(factor))
        .ok_or(DurationError::CalendarOverflow)?;
    i64::try_from(scaled).map_err(|source| DurationError::SecondsOverflow {
        spec: spec.to_owned(),
        source,
    })
}

fn month_seconds(amount: u64, current: civil::DateTime) -> Result<i64, DurationError> {
    let target = month_target(amount, current)?;
    let days = current.date().duration_until(target.date());
    Ok(days.as_secs())
}

fn month_target(amount: u64, current: civil::DateTime) -> Result<civil::DateTime, DurationError> {
    let month_index = current
        .month()
        .checked_sub(1)
        .ok_or(DurationError::CalendarOverflow)?;
    let month_offset =
        u64::try_from(month_index).map_err(|source| DurationError::CalendarField { source })?;
    let total_months = amount
        .checked_add(month_offset)
        .ok_or(DurationError::CalendarOverflow)?;
    let years = total_months
        .checked_div(12)
        .ok_or(DurationError::CalendarOverflow)?;
    let months = total_months
        .checked_rem(12)
        .ok_or(DurationError::CalendarOverflow)?;
    let target_year = i128::from(current.year())
        .checked_add(i128::from(years))
        .ok_or(DurationError::CalendarOverflow)?;
    let target_month = u8::try_from(months)
        .map_err(|source| DurationError::CalendarField { source })?
        .checked_add(1)
        .ok_or(DurationError::CalendarOverflow)?;
    let year =
        i16::try_from(target_year).map_err(|source| DurationError::CalendarField { source })?;
    let month =
        i8::try_from(target_month).map_err(|source| DurationError::CalendarField { source })?;
    let first =
        civil::Date::new(year, month, 1).map_err(|source| DurationError::Calendar { source })?;
    let day = current.day().min(first.days_in_month());
    civil::DateTime::new(
        year,
        month,
        day,
        current.hour(),
        current.minute(),
        current.second(),
        current.subsec_nanosecond(),
    )
    .map_err(|source| DurationError::Calendar { source })
}
