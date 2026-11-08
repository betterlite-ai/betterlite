use betterlite_core::agents::json::AgentJson;

use crate::version_convert::{JsonObject, to_object};

use super::error::ConvertError;

type Accessor<T> = Result<Option<T>, ConvertError>;

#[derive(Clone, Copy)]
pub(super) enum Copy {
    NonEmptyText,
    TextOrDefault,
    Object,
}

pub(super) fn string(fields: &JsonObject, key: &str) -> Accessor<String> {
    match fields.get(key) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(field_error(key, "a string")),
    }
}

pub(super) fn object(fields: &JsonObject, key: &str) -> Accessor<JsonObject> {
    match fields.get(key) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(value) => to_object(value)
            .map(Some)
            .ok_or_else(|| field_error(key, "an object")),
    }
}

pub(super) fn array(fields: &JsonObject, key: &str) -> Accessor<Vec<AgentJson>> {
    match fields.get(key) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::Array(items)) => Ok(Some(items.clone())),
        Some(_) => Err(field_error(key, "an array")),
    }
}

pub(super) fn boolean(fields: &JsonObject, key: &str) -> Accessor<bool> {
    match fields.get(key) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(field_error(key, "a boolean")),
    }
}

pub(super) fn integer(fields: &JsonObject, key: &str) -> Accessor<i64> {
    match fields.get(key) {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::Number(number)) => integer_of(number, key),
        Some(_) => Err(field_error(key, "an integer")),
    }
}

fn integer_of(number: &serde_json::Number, key: &str) -> Accessor<i64> {
    number
        .as_i64()
        .map(Some)
        .ok_or_else(|| field_error(key, "an integer"))
}

pub(super) fn insert(fields: &mut JsonObject, key: &str, value: Option<AgentJson>) {
    if let Some(present) = value {
        fields.insert(key.to_owned(), present);
    }
}

pub(super) fn copy(
    mode: Copy,
    target: &mut JsonObject,
    target_key: &str,
    source: &JsonObject,
    source_key: &str,
) -> Result<(), ConvertError> {
    let value = match mode {
        Copy::NonEmptyText => string(source, source_key)?
            .filter(|text| !text.is_empty())
            .map(AgentJson::String),
        Copy::TextOrDefault => Some(AgentJson::String(
            string(source, source_key)?.unwrap_or_default(),
        )),
        Copy::Object => object(source, source_key)?
            .map(|fields| AgentJson::Object(fields.into_iter().collect())),
    };
    insert(target, target_key, value);
    Ok(())
}

fn field_error(field: &str, expected: &'static str) -> ConvertError {
    ConvertError::Field {
        field: field.to_owned(),
        expected,
    }
}
