use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, SecondsFormat};

// A full ISO datetime needs 35 characters at most, so a longer string is refused before it is parsed
pub const MAX_BUDGET_RESET_CHARS: usize = 64;

const NAIVE_FORMATS: [&str; 2] = ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BudgetResetError {
    #[error("the value is {length} characters, past the {MAX_BUDGET_RESET_CHARS} a datetime needs")]
    TooLong { length: usize },
    #[error("the value is not an ISO 8601 datetime")]
    Invalid,
}

// Python's fromisoformat reads a bare Z, so LiteLLM rewrites it to +00:00 before parsing
pub fn decode_budget_reset(value: &str) -> Result<String, BudgetResetError> {
    let length = value.chars().count();
    if length > MAX_BUDGET_RESET_CHARS {
        return Err(BudgetResetError::TooLong { length });
    }
    let normalized = value.replace('Z', "+00:00");
    if let Ok(moment) = DateTime::parse_from_rfc3339(&normalized) {
        return Ok(isoformat(moment));
    }
    for format in NAIVE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, format) {
            return Ok(isoformat(naive.and_utc().fixed_offset()));
        }
    }
    let midnight = NaiveDate::parse_from_str(&normalized, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0));
    midnight.map_or(Err(BudgetResetError::Invalid), |date_time| {
        Ok(isoformat(date_time.and_utc().fixed_offset()))
    })
}

// Python's isoformat writes six fractional digits only when the microsecond is set, and +00:00 for UTC
fn isoformat(moment: DateTime<FixedOffset>) -> String {
    let precision = if moment.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    moment.to_rfc3339_opts(precision, false)
}

mod tests;
