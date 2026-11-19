use std::collections::BTreeMap;

use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::duration::error::DurationError;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use chrono::{DateTime, Duration, Utc};
use jiff::tz::TimeZoneDatabase;

pub mod alias;
pub mod columns;
pub mod generate;
pub mod hooks;
pub mod key_type;
pub mod model_budget;
pub mod regenerate;
pub mod rotation;
pub mod unique_alias;

mod tests;

// the injected inputs of the key helpers: the request time, the reset policy and the calendar data
#[derive(Clone, Copy)]
pub struct KeyClock<'context> {
    pub now: DateTime<Utc>,
    pub reset: &'context BudgetResetSettings,
    pub database: &'context TimeZoneDatabase,
}

#[derive(Debug, thiserror::Error)]
pub enum ClockError {
    #[error("{spec} is not a duration")]
    Duration {
        spec: String,
        #[source]
        source: DurationError,
    },
    #[error("the {spec} duration does not fit the request time")]
    Overflow { spec: String },
}

// the same text pydantic writes in JSON, where a UTC offset is the letter Z
#[must_use]
pub fn zulu_datetime(instant: DateTime<Utc>) -> String {
    let text = isoformat(instant);
    let Some(head) = text.strip_suffix("+00:00") else {
        return text;
    };
    format!("{head}Z")
}

// a Python datetime holds microseconds, so a request time with finer digits is cut to them first
#[must_use]
pub fn microsecond_precision(instant: DateTime<Utc>) -> DateTime<Utc> {
    let nanoseconds = instant.timestamp_subsec_micros().saturating_mul(1_000);
    DateTime::from_timestamp(instant.timestamp(), nanoseconds).unwrap_or(instant)
}

// every key alias rejection LiteLLM raises is this one ProxyException shape
pub fn bad_request_for_alias(message: String) -> Box<ProxyException> {
    Box::new(ProxyException::new(ProxyExceptionInit {
        message,
        error_type: ProxyErrorTypes::BadRequestError.to_string(),
        param: Some("key_alias".to_owned()),
        code: ExceptionCode::Status(400),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    }))
}

// now plus a duration spec, the shape of every expires, reset and rotation time this component computes
pub fn shifted_by(
    now: DateTime<Utc>,
    spec: &str,
    clock: KeyClock<'_>,
) -> Result<DateTime<Utc>, ClockError> {
    let parsed = duration_in_seconds(spec, now, None, clock.database).map_err(|source| {
        ClockError::Duration {
            spec: spec.to_owned(),
            source,
        }
    })?;
    let offset = Duration::try_seconds(parsed.seconds).ok_or_else(|| ClockError::Overflow {
        spec: spec.to_owned(),
    })?;
    microsecond_precision(now)
        .checked_add_signed(offset)
        .ok_or_else(|| ClockError::Overflow {
            spec: spec.to_owned(),
        })
}
