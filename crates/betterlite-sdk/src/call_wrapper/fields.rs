use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::{CallError, field_error};
use crate::call_setup::RequestMap;

// absent and null both take the declared default, and a value of the wrong type names its field
pub fn text_field(request: &RequestMap, field: &'static str) -> Result<Option<String>, CallError> {
    match request.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(field_error(field, "a string"))
        }
    }
}

pub fn int_field(request: &RequestMap, field: &'static str) -> Result<Option<i64>, CallError> {
    match request.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| field_error(field, "an integer")),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(field_error(field, "an integer"))
        }
    }
}

pub fn bool_field(request: &RequestMap, field: &'static str) -> Result<Option<bool>, CallError> {
    match request.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(field_error(field, "a boolean"))
        }
    }
}

pub fn object_field<'request>(
    request: &'request RequestMap,
    field: &'static str,
) -> Result<Option<&'request IndexMap<String, Json>>, CallError> {
    match request.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(field_error(field, "an object"))
        }
    }
}
