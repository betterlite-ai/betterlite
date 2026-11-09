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

pub(super) fn top_level_count(
    value: Option<&Json>,
    field: &'static str,
) -> Result<i64, UsageShapeError> {
    match value {
        Some(Json::Bool(flag)) => Ok(i64::from(*flag)),
        Some(Json::Number(number)) => truncated_number(number, field),
        None | Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) => Ok(0),
    }
}

pub(super) fn declared_count(
    value: Option<&Json>,
    field: &'static str,
) -> Result<i64, UsageShapeError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(flag)) => Ok(i64::from(*flag)),
        Some(Json::Number(number)) => integer_value(number, field),
        Some(Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(UsageShapeError::FieldType { field })
        }
    }
}

pub(super) fn optional_count(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<i64>, UsageShapeError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(_) => declared_count(value, field).map(Some),
    }
}

pub(super) fn integer_value(number: &Number, field: &'static str) -> Result<i64, UsageShapeError> {
    if number.as_i64().is_none() && number.as_u64().is_none() {
        return Err(UsageShapeError::FieldType { field });
    }
    truncated_number(number, field)
}

pub(super) fn coerced_count(
    value: Option<&Json>,
    field: &'static str,
) -> Result<i64, UsageShapeError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Bool(flag)) => Ok(i64::from(*flag)),
        Some(Json::Number(number)) => truncated_number(number, field),
        Some(Json::Text(text)) if text.is_empty() => Ok(0),
        Some(Json::Text(text)) => text
            .trim()
            .parse::<i64>()
            .ok()
            .ok_or(UsageShapeError::FieldType { field }),
        Some(Json::List(items)) if items.is_empty() => Ok(0),
        Some(Json::Object(entries)) if entries.is_empty() => Ok(0),
        Some(Json::List(_) | Json::Object(_)) => Err(UsageShapeError::FieldType { field }),
    }
}

pub(super) fn optional_text(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, UsageShapeError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(UsageShapeError::FieldType { field: name })
        }
    }
}

pub(super) fn resolved_speed(
    fields: &IndexMap<String, Json>,
    speed: Option<&str>,
) -> Option<String> {
    match fields.get("speed") {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => speed.map(str::to_owned),
    }
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
