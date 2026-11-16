use chrono::{DateTime, Datelike as _, Days, NaiveDate, Utc};
use http::StatusCode;
use thiserror::Error;

use super::USAGE_MAX_RANGE_DAYS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageWindow {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UsageWindowError {
    #[error("start_date and end_date must be in YYYY-MM-DD format")]
    DateFormat,
    #[error("start_date must be on or before end_date")]
    OutOfOrder,
    #[error("Date range too large; maximum is {USAGE_MAX_RANGE_DAYS} days")]
    RangeTooLarge,
}

impl UsageWindowError {
    #[must_use]
    pub const fn status_code(self) -> StatusCode {
        match self {
            Self::DateFormat | Self::OutOfOrder | Self::RangeTooLarge => StatusCode::BAD_REQUEST,
        }
    }

    #[must_use]
    pub const fn error_type(self) -> &'static str {
        match self {
            Self::DateFormat | Self::OutOfOrder | Self::RangeTooLarge => "bad_request",
        }
    }
}

fn parse_usage_date(text: &str) -> Option<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    let year = parsed.year();
    if !(1_i32..=9_999_i32).contains(&year) {
        return None;
    }
    // chrono's parser takes one-digit months and signed years, which Python's fromisoformat refuses
    (parsed.format("%Y-%m-%d").to_string() == text).then_some(parsed)
}

pub fn resolve_usage_window(
    start_date: Option<&str>,
    end_date: Option<&str>,
    now: DateTime<Utc>,
) -> Result<UsageWindow, UsageWindowError> {
    let today = now.date_naive();
    let end = end_date.map_or_else(|| today.format("%Y-%m-%d").to_string(), str::to_owned);
    // chrono's calendar starts far below any clock date, so only its own first day overflows
    let week_ago = today
        .checked_sub_days(Days::new(7))
        .unwrap_or(NaiveDate::MIN);
    let start = start_date.map_or_else(|| week_ago.format("%Y-%m-%d").to_string(), str::to_owned);
    let Some(start_parsed) = parse_usage_date(&start) else {
        return Err(UsageWindowError::DateFormat);
    };
    let Some(end_parsed) = parse_usage_date(&end) else {
        return Err(UsageWindowError::DateFormat);
    };
    if end_parsed < start_parsed {
        return Err(UsageWindowError::OutOfOrder);
    }
    if end_parsed.signed_duration_since(start_parsed).num_days() > USAGE_MAX_RANGE_DAYS {
        return Err(UsageWindowError::RangeTooLarge);
    }
    Ok(UsageWindow { start, end })
}

mod tests;
