use betterlite_models::decisions::Json;
use num_traits::cast;
use serde_json::Number;

use crate::converse_usage::error::ConverseUsageError;
use crate::converse_usage::wire::ConverseTokenUsage;

type Counted = Result<Option<i64>, ConverseUsageError>;
type Count = Result<i64, ConverseUsageError>;

// LiteLLM's isinstance(value, (int, float)) guard: a number truncates toward zero and any other shape is skipped.
pub(super) fn count_value(value: Option<&Json>, field: &'static str) -> Counted {
    match value {
        Some(Json::Number(number)) => number_value(number, field, true).map(Some),
        _ => Ok(None),
    }
}

pub(super) fn first_count(aliases: &[(&'static str, &Option<Json>)]) -> Count {
    for (field, value) in aliases {
        if let Some(count) = count_value(value.as_ref(), field)? {
            return Ok(count);
        }
    }
    Ok(0)
}

pub(super) fn cache_counts(usage: &ConverseTokenUsage) -> Result<(i64, i64), ConverseUsageError> {
    let read = first_count(&[
        ("cacheReadInputTokens", &usage.cache_read_input_tokens),
        (
            "cacheReadInputTokenCount",
            &usage.cache_read_input_token_count,
        ),
    ])?;
    let write = first_count(&[
        ("cacheWriteInputTokens", &usage.cache_write_input_tokens),
        (
            "cacheWriteInputTokenCount",
            &usage.cache_write_input_token_count,
        ),
    ])?;
    Ok((read, write))
}

// A live inputTokens or outputTokens is a declared int, so a float or a bool is refused here rather than truncated.
pub(super) fn live_count(value: Option<&Json>, field: &'static str) -> Count {
    match value {
        Some(Json::Number(number)) => number_value(number, field, false),
        _ => Err(ConverseUsageError::FieldType { field }),
    }
}

pub(super) fn number_value(number: &Number, field: &'static str, truncate: bool) -> Count {
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
