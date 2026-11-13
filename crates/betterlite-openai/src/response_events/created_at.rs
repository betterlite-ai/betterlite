use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("created_at cannot be converted into an integer timestamp")]
pub struct CreatedAtError;

// LiteLLM's _safe_convert_created_field: int(float(value)) for anything but a plain number
pub fn convert_created_at(value: &Json, now: i64) -> Result<i64, CreatedAtError> {
    match value {
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => Ok(now),
        Json::Number(number) => number.as_i64().map_or_else(
            || {
                number
                    .as_f64()
                    .map_or(Ok(now), |float| convert_float(float, now))
            },
            Ok,
        ),
        Json::Text(text) => convert_text(text, now),
    }
}

fn convert_text(text: &str, now: i64) -> Result<i64, CreatedAtError> {
    let Ok(number) = text.trim().parse::<f64>() else {
        return Ok(now);
    };
    convert_float(number, now)
}

fn convert_float(number: f64, now: i64) -> Result<i64, CreatedAtError> {
    if number.is_nan() {
        return Ok(now);
    }
    // int(inf) raises OverflowError, which the source does not catch into its current-time fallback
    if number.is_infinite() {
        return Err(CreatedAtError);
    }
    let Some(decimal) = Decimal::from_f64(number) else {
        return Ok(now);
    };
    Ok(decimal.to_i64().unwrap_or(now))
}
