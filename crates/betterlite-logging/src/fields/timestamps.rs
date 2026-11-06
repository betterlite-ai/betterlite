use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// A logging timestamp: a datetime, a float of epoch seconds, or an int that cleanup_timestamps rejects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    DateTime(DateTime<FixedOffset>),
    Float(Decimal),
    Integer(i64),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Timestamps {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub start_time: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub end_time: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub completion_start_time: Decimal,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum TimestampError {
    #[error("start_time is required and must be a datetime or a float of epoch seconds")]
    StartTimeRequired,
    #[error("end_time is required and must be a datetime or a float of epoch seconds")]
    EndTimeRequired,
    #[error("the response time overflowed the range of a decimal")]
    ResponseTimeOverflow,
}

// StandardLoggingPayloadSetup.cleanup_timestamps: completion_start_time falls back to end_time unless it is a datetime or float.
pub fn cleanup_timestamps(
    start: Option<&Moment>,
    end: Option<&Moment>,
    completion_start: Option<&Moment>,
) -> Result<Timestamps, TimestampError> {
    let start_time = accepted_seconds(start).ok_or(TimestampError::StartTimeRequired)?;
    let end_time = accepted_seconds(end).ok_or(TimestampError::EndTimeRequired)?;
    let completion_start_time = accepted_seconds(completion_start).unwrap_or(end_time);
    Ok(Timestamps {
        start_time,
        end_time,
        completion_start_time,
    })
}

// _timestamp_seconds: a datetime or any number is epoch seconds; a value outside those shapes never reaches it.
#[must_use]
pub fn timestamp_seconds(moment: &Moment) -> Decimal {
    match moment {
        Moment::DateTime(datetime) => with_fraction(Decimal::new(datetime.timestamp_micros(), 6)),
        Moment::Float(value) => with_fraction(*value),
        Moment::Integer(value) => with_fraction(Decimal::from(*value)),
    }
}

// Logging.get_response_ms, with the current moment injected for the missing-timestamp fallback.
pub fn get_response_ms(
    start: Option<&Moment>,
    end: Option<&Moment>,
    now: &Moment,
) -> Result<Decimal, TimestampError> {
    let start_seconds = timestamp_seconds(start.unwrap_or(now));
    let end_seconds = timestamp_seconds(end.unwrap_or(now));
    let elapsed = end_seconds
        .checked_sub(start_seconds)
        .ok_or(TimestampError::ResponseTimeOverflow)?;
    let millis = elapsed
        .checked_mul(Decimal::from(1_000_i32))
        .ok_or(TimestampError::ResponseTimeOverflow)?;
    Ok(with_fraction(millis))
}

// Python's json.dumps writes every float with a decimal point, so a whole number of seconds keeps one digit.
pub(crate) fn with_fraction(value: Decimal) -> Decimal {
    let mut normalized = value.normalize();
    if normalized.scale() == 0 {
        normalized.rescale(1);
    }
    normalized
}

// cleanup_timestamps accepts only a datetime or a float; an int is a declared-type error there.
fn accepted_seconds(moment: Option<&Moment>) -> Option<Decimal> {
    match moment {
        Some(Moment::DateTime(datetime)) => {
            Some(with_fraction(Decimal::new(datetime.timestamp_micros(), 6)))
        }
        Some(Moment::Float(value)) => Some(with_fraction(*value)),
        Some(Moment::Integer(_)) | None => None,
    }
}

mod tests;
