use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

use crate::budget_primitives::window_start::coerce_datetime;

pub const MAX_TEMP_BUDGET_EXPIRY_BYTES: usize = 0x0010_0000;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TempBudgetError {
    #[error("the temporary budget expiry exceeds {MAX_TEMP_BUDGET_EXPIRY_BYTES} bytes")]
    ExpiryTooLong,
    #[error("the temporary budget expiry is not an ISO 8601 timestamp")]
    ExpiryInvalid,
    #[error("the temporary budget increase is not a number")]
    IncreaseInvalid,
}

// datetime.fromisoformat with the Z spelling and a naive value read as UTC, on bounded input
pub fn parse_temp_budget_expiry(text: &str) -> Result<DateTime<FixedOffset>, TempBudgetError> {
    if text.len() > MAX_TEMP_BUDGET_EXPIRY_BYTES {
        return Err(TempBudgetError::ExpiryTooLong);
    }
    let normalized = normalize_basic_iso8601(text);
    coerce_datetime(Some(&JsonData::String(normalized))).ok_or(TempBudgetError::ExpiryInvalid)
}

// fromisoformat also reads the ISO 8601 basic forms, which the shared parser does not
fn normalize_basic_iso8601(text: &str) -> String {
    let Some(position) = text.find(['T', 't', ' ', '_']) else {
        return normalize_basic_date(text);
    };
    let (date, rest) = text.split_at(position);
    let (separator, time) = rest.split_at(1);
    format!(
        "{}{separator}{}",
        normalize_basic_date(date),
        normalize_basic_time(time)
    )
}

fn normalize_basic_date(date: &str) -> String {
    if date.len() != 8 {
        return date.to_owned();
    }
    if !date.bytes().all(|byte| byte.is_ascii_digit()) {
        return date.to_owned();
    }
    let (year, rest) = date.split_at(4);
    let (month, day) = rest.split_at(2);
    format!("{year}-{month}-{day}")
}

fn normalize_basic_time(time: &str) -> String {
    let end = time.find(['.', 'Z', 'z', '+', '-']).unwrap_or(time.len());
    let (digits, tail) = time.split_at(end);
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return time.to_owned();
    }
    let normalized = match digits.len() {
        6 => {
            let (hours, rest) = digits.split_at(2);
            let (minutes, seconds) = rest.split_at(2);
            format!("{hours}:{minutes}:{seconds}")
        }
        4 => {
            let (hours, minutes) = digits.split_at(2);
            format!("{hours}:{minutes}")
        }
        2 => format!("{digits}:00"),
        _ => return time.to_owned(),
    };
    format!("{normalized}{tail}")
}
