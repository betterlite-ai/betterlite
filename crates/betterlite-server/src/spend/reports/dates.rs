use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use thiserror::Error;

const MAX_RANGE_DAYS: i64 = 366;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SpendReportDates {
    pub(super) start: DateTime<Utc>,
    pub(super) end: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(super) enum DateRangeError {
    #[error("Please provide start_date and end_date")]
    Missing,
    #[error("start_date and end_date must be in YYYY-MM-DD format")]
    Format,
    #[error("start_date must be on or before end_date")]
    Reversed,
    #[error("Date range too large; maximum is {MAX_RANGE_DAYS} days")]
    TooLarge,
}

pub(super) fn spend_report_dates(
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<SpendReportDates, DateRangeError> {
    let (Some(start_text), Some(end_text)) = (start_date, end_date) else {
        return Err(DateRangeError::Missing);
    };
    let start = parse_date(start_text).ok_or(DateRangeError::Format)?;
    let end = parse_date(end_text).ok_or(DateRangeError::Format)?;
    if end < start {
        return Err(DateRangeError::Reversed);
    }
    if end.signed_duration_since(start) > TimeDelta::days(MAX_RANGE_DAYS) {
        return Err(DateRangeError::TooLarge);
    }
    Ok(SpendReportDates { start, end })
}

// CPython strptime("%Y-%m-%d"): a four digit year, 1-2 digit month and day, no surrounding text.
fn parse_date(value: &str) -> Option<DateTime<Utc>> {
    let (year_text, remainder) = value.split_once('-')?;
    let (month_text, day_text) = remainder.split_once('-')?;
    let year = parse_year(year_text)?;
    let month = parse_month(month_text)?;
    let day = parse_day(day_text)?;
    NaiveDate::from_ymd_opt(year, month, day).map(|date| date.and_time(NaiveTime::MIN).and_utc())
}

fn parse_year(value: &str) -> Option<i32> {
    if value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = value.parse::<i32>().ok()?;
    (year >= 1_i32).then_some(year)
}

fn parse_month(value: &str) -> Option<u32> {
    if value.len() > 2 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let month = value.parse::<u32>().ok()?;
    (1_u32..=12).contains(&month).then_some(month)
}

// CPython's %d also takes one space before a single non-zero digit
fn parse_day(value: &str) -> Option<u32> {
    let digits = value.strip_prefix(' ').unwrap_or(value);
    let width = if digits.len() == value.len() { 2 } else { 1 };
    if digits.is_empty()
        || digits.len() > width
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let day = digits.parse::<u32>().ok()?;
    (1_u32..=31).contains(&day).then_some(day)
}
