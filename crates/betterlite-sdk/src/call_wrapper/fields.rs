use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::{CallError, field_error};
use crate::call_setup::RequestMap;

// One reader per declared value type: absent and null take the default, a wrong type names its field
pub trait FieldValue: Sized {
    fn read(field: &'static str, value: &Json) -> Result<Self, CallError>;
}

impl FieldValue for String {
    fn read(field: &'static str, value: &Json) -> Result<Self, CallError> {
        if let Json::Text(text) = value {
            return Ok(text.clone());
        }
        Err(field_error(field, "a string"))
    }
}

impl FieldValue for i64 {
    fn read(field: &'static str, value: &Json) -> Result<Self, CallError> {
        let Json::Number(number) = value else {
            return Err(field_error(field, "an integer"));
        };
        number
            .as_i64()
            .ok_or_else(|| field_error(field, "an integer"))
    }
}

impl FieldValue for bool {
    fn read(field: &'static str, value: &Json) -> Result<Self, CallError> {
        if let Json::Bool(flag) = value {
            return Ok(*flag);
        }
        Err(field_error(field, "a boolean"))
    }
}

pub fn field<T>(request: &RequestMap, name: &'static str) -> Result<Option<T>, CallError>
where
    T: FieldValue,
{
    let Some(value) = request.get(name) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    T::read(name, value).map(Some)
}

pub fn object_field<'request>(
    request: &'request RequestMap,
    name: &'static str,
) -> Result<Option<&'request IndexMap<String, Json>>, CallError> {
    match request.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(field_error(name, "an object")),
    }
}
