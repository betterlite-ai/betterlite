use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::cast;
use serde_json::Number;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum UsageShapeError {
    #[error("the usage field {field} has a type the declared Anthropic shape does not allow")]
    FieldType { field: &'static str },
    #[error("the usage field {field} is outside the signed 64-bit range")]
    OutOfRange { field: &'static str },
    #[error("the usage arithmetic overflowed at {field}")]
    Overflow { field: &'static str },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CountMode {
    // LiteLLM turns any non numeric top-level count into zero instead of failing the response.
    Lenient,
    // A count the Anthropic TypedDicts declare as int takes an integer, or a boolean.
    Declared,
    // int(value or 0) truncates numbers and reads integer text.
    Coerced,
}

pub(super) fn count_value(
    value: Option<&Json>,
    field: &'static str,
    mode: CountMode,
) -> Result<i64, UsageShapeError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(flag)) => Ok(i64::from(*flag)),
        Some(Json::Number(number)) => number_value(number, field, mode),
        Some(Json::Text(text)) => count_text(text, field, mode),
        Some(Json::List(items)) => count_empty(items.is_empty(), field, mode),
        Some(Json::Object(entries)) => count_empty(entries.is_empty(), field, mode),
    }
}

pub(super) fn optional_count(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<i64>, UsageShapeError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(_) => count_value(value, field, CountMode::Declared).map(Some),
    }
}

pub(super) fn integer_value(number: &Number, field: &'static str) -> Result<i64, UsageShapeError> {
    if number.as_i64().is_none() && number.as_u64().is_none() {
        return Err(UsageShapeError::FieldType { field });
    }
    truncated_number(number, field)
}

pub(super) fn optional_text(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, UsageShapeError> {
    let Some(value) = fields.get(name) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(UsageShapeError::FieldType { field: name })
        }
    }
}

pub(super) fn resolved_speed(
    fields: &IndexMap<String, Json>,
    speed: Option<&str>,
) -> Option<String> {
    if let Some(Json::Text(text)) = fields.get("speed") {
        return Some(text.clone());
    }
    speed.map(str::to_owned)
}

fn number_value(
    number: &Number,
    field: &'static str,
    mode: CountMode,
) -> Result<i64, UsageShapeError> {
    if mode == CountMode::Declared {
        return integer_value(number, field);
    }
    truncated_number(number, field)
}

fn count_text(text: &str, field: &'static str, mode: CountMode) -> Result<i64, UsageShapeError> {
    if mode == CountMode::Lenient {
        return Ok(0);
    }
    if mode == CountMode::Declared {
        return Err(UsageShapeError::FieldType { field });
    }
    if text.is_empty() {
        return Ok(0);
    }
    text.trim()
        .parse::<i64>()
        .ok()
        .ok_or(UsageShapeError::FieldType { field })
}

fn count_empty(empty: bool, field: &'static str, mode: CountMode) -> Result<i64, UsageShapeError> {
    if empty || mode == CountMode::Lenient {
        return Ok(0);
    }
    Err(UsageShapeError::FieldType { field })
}

fn truncated_number(number: &Number, field: &'static str) -> Result<i64, UsageShapeError> {
    if let Some(integer) = number.as_i64() {
        return Ok(integer);
    }
    if let Some(integer) = number.as_u64() {
        return i64::try_from(integer)
            .ok()
            .ok_or(UsageShapeError::OutOfRange { field });
    }
    number
        .as_f64()
        .and_then(cast::<f64, i64>)
        .ok_or(UsageShapeError::OutOfRange { field })
}
