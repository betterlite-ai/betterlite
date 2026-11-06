use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Number;

use super::PayloadError;

pub(super) fn micros(moment: DateTime<Utc>) -> Result<i64, PayloadError> {
    moment
        .timestamp()
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(i64::from(moment.timestamp_subsec_micros())))
        .ok_or(PayloadError::Field {
            field: "timestamp",
            expected: "a representable instant",
        })
}

pub(super) fn difference(
    later: DateTime<Utc>,
    earlier: DateTime<Utc>,
    field: &'static str,
) -> Result<Json, PayloadError> {
    let elapsed = Decimal::new(micros(later)?, 6)
        .checked_sub(Decimal::new(micros(earlier)?, 6))
        .ok_or(PayloadError::Field {
            field,
            expected: "a representable duration",
        })?;
    text(elapsed, field)
}

// Python writes a timestamp or duration as the shortest decimal that round-trips, with a fraction digit.
pub(super) fn text(seconds: Decimal, field: &'static str) -> Result<Json, PayloadError> {
    let normalized = seconds.normalize().to_string();
    let value = if normalized.contains('.') {
        normalized
    } else {
        format!("{normalized}.0")
    };
    serde_json::from_str::<Number>(&value)
        .map(Json::Number)
        .map_err(|_source| PayloadError::Field {
            field,
            expected: "a JSON number",
        })
}
