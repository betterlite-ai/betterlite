use betterlite_models::decisions::Json;
use num_traits::cast;
use serde_json::Number;

use crate::converse_usage::error::ConverseUsageError;
use crate::converse_usage::wire::ConverseTokenUsage;

// LiteLLM's isinstance(value, (int, float)) guard: a number truncates toward zero and any other shape is skipped.
pub(super) fn count_value(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<i64>, ConverseUsageError> {
    match value {
        Some(Json::Number(number)) => number_value(number, field, true).map(Some),
        None
        | Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) | Json::Null) => {
            Ok(None)
        }
    }
}

pub(super) fn first_count(
    fields: &[(&'static str, &Option<Json>)],
) -> Result<i64, ConverseUsageError> {
    for (field, value) in fields {
        if let Some(count) = count_value(value.as_ref(), field)? {
            return Ok(count);
        }
    }
    Ok(0)
}

pub(super) fn cache_read_count(usage: &ConverseTokenUsage) -> Result<i64, ConverseUsageError> {
    first_count(&[
        ("cacheReadInputTokens", &usage.cache_read_input_tokens),
        (
            "cacheReadInputTokenCount",
            &usage.cache_read_input_token_count,
        ),
    ])
}

pub(super) fn cache_write_count(usage: &ConverseTokenUsage) -> Result<i64, ConverseUsageError> {
    first_count(&[
        ("cacheWriteInputTokens", &usage.cache_write_input_tokens),
        (
            "cacheWriteInputTokenCount",
            &usage.cache_write_input_token_count,
        ),
    ])
}

// A live inputTokens or outputTokens is a declared int, so a float or a bool is refused here rather than truncated.
pub(super) fn live_count(
    value: Option<&Json>,
    field: &'static str,
) -> Result<i64, ConverseUsageError> {
    let Some(Json::Number(number)) = value else {
        return Err(ConverseUsageError::FieldType { field });
    };
    number_value(number, field, false)
}

pub(super) fn number_value(
    number: &Number,
    field: &'static str,
    truncate: bool,
) -> Result<i64, ConverseUsageError> {
    if let Some(count) = number.as_i64() {
        return Ok(count);
    }
    if number.as_u64().is_some() {
        return Err(ConverseUsageError::OutOfRange { field });
    }
    if truncate && let Some(float) = number.as_f64() {
        return cast::<f64, i64>(float).ok_or(ConverseUsageError::OutOfRange { field });
    }
    Err(ConverseUsageError::FieldType { field })
}

// The provider thinking count is an int | None field, so only an integer literal or a boolean reads; every other shape is None.
pub(super) fn thinking_number(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => integer_literal(number),
        Some(Json::Bool(flag)) => Some(i64::from(*flag)),
        None | Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

fn integer_literal(number: &Number) -> Option<i64> {
    if let Some(count) = number.as_i64() {
        return Some(count);
    }
    let count = number.as_u64()?;
    i64::try_from(count).ok()
}
