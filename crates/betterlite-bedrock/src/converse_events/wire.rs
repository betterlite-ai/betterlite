use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::cast;
use serde_json::Number;

use crate::converse_events::error::ConverseEventError;

macro_rules! json_accessor {
    ($name:ident, $variant:ident, $type:ty, $convert:ident) => {
        pub const fn $name<'value>(
            value: &'value Json,
            field: &'static str,
        ) -> Result<&'value $type, ConverseEventError> {
            match value {
                Json::$variant(inner) => Ok(inner.$convert()),
                _ => Err(ConverseEventError::FieldType { field }),
            }
        }
    };
}

pub const fn object<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value IndexMap<String, Json>, ConverseEventError> {
    let Json::Object(fields) = value else {
        return Err(ConverseEventError::FieldType { field });
    };
    Ok(fields)
}

json_accessor!(list, List, [Json], as_slice);
json_accessor!(text, Text, str, as_str);

pub fn required_text<'value>(
    fields: &'value IndexMap<String, Json>,
    key: &str,
    field: &'static str,
) -> Result<&'value str, ConverseEventError> {
    let value = fields
        .get(key)
        .ok_or(ConverseEventError::Missing { field })?;
    text(value, field)
}

pub fn optional_text(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, ConverseEventError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseEventError::FieldType { field })
        }
    }
}

// LiteLLM int()s the index, so a float truncates toward zero and a missing one is zero.
pub fn content_index(value: Option<&Json>) -> Result<i64, ConverseEventError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => index_number(number),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseEventError::FieldType {
                field: "contentBlockIndex",
            })
        }
    }
}

fn index_number(number: &Number) -> Result<i64, ConverseEventError> {
    if let Some(count) = number.as_i64() {
        return Ok(count);
    }
    if number.as_u64().is_some() {
        return Err(ConverseEventError::IndexRange);
    }
    number
        .as_f64()
        .and_then(cast::<f64, i64>)
        .ok_or(ConverseEventError::IndexRange)
}

// LiteLLM tests trace and completionReason with bool(), so null, false, zero, an empty string, list or object are all absent.
#[must_use]
pub fn non_empty(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|real| real != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
