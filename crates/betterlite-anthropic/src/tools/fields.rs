use betterlite_prompts::arguments::json::{Json, Object};

use super::ToolsError;

// absent and null both mean the caller left the field out, so a typed reader never sees either
pub(super) fn field<'fields, T>(
    fields: &'fields Object,
    key: &'static str,
    expected: &'static str,
    extract: fn(&'fields Json) -> Option<T>,
) -> Result<Option<T>, ToolsError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => extract(value).map(Some).ok_or(ToolsError::WrongType {
            field: key,
            expected,
        }),
    }
}

pub(super) const fn number(value: &Json) -> Option<&Json> {
    match value {
        Json::Number(_) => Some(value),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) const fn non_null(value: Option<&Json>) -> Option<&Json> {
    match value {
        None | Some(Json::Null) => None,
        Some(present) => Some(present),
    }
}

pub(super) fn function_fields(fields: &Object) -> Result<Option<&Object>, ToolsError> {
    match fields.get("function") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(function)) => Ok(Some(function)),
        Some(_) => Err(ToolsError::WrongType {
            field: "function",
            expected: "an object",
        }),
    }
}

pub(super) fn string_list(value: &Json, field: &'static str) -> Result<Vec<String>, ToolsError> {
    let Json::List(items) = value else {
        return Err(ToolsError::WrongType {
            field,
            expected: "a list of strings",
        });
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or(ToolsError::WrongType {
                    field,
                    expected: "a list of strings",
                })
        })
        .collect()
}
