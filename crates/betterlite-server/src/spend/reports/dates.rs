use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;

use crate::cache_activity_routes::date::parse_date;

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
