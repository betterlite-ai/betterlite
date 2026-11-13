use betterlite_prompts::arguments::json::{Json, Object};

use super::ToolsError;

pub(super) fn required_string<'fields>(
    fields: &'fields Object,
    key: &'static str,
) -> Result<&'fields str, ToolsError> {
    match fields.get(key) {
        Some(Json::Text(text)) => Ok(text.as_str()),
        None | Some(Json::Null) => Err(ToolsError::Missing(key)),
        Some(_) => Err(ToolsError::WrongType {
            field: key,
            expected: "a string",
        }),
    }
}

pub(super) fn optional_string<'fields>(
    fields: &'fields Object,
    key: &'static str,
) -> Result<Option<&'fields str>, ToolsError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(ToolsError::WrongType {
            field: key,
            expected: "a string",
        }),
    }
}

pub(super) fn required_number<'fields>(
    fields: &'fields Object,
    key: &'static str,
) -> Result<&'fields Json, ToolsError> {
    match fields.get(key) {
        Some(number @ Json::Number(_)) => Ok(number),
        None | Some(Json::Null) => Err(ToolsError::Missing(key)),
        Some(_) => Err(ToolsError::WrongType {
            field: key,
            expected: "a number",
        }),
    }
}

pub(super) fn optional_number<'fields>(
    fields: &'fields Object,
    key: &'static str,
) -> Result<Option<&'fields Json>, ToolsError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(number @ Json::Number(_)) => Ok(Some(number)),
        Some(_) => Err(ToolsError::WrongType {
            field: key,
            expected: "a number",
        }),
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

pub(super) fn string_list(value: &Json) -> Result<Vec<String>, ToolsError> {
    let Json::List(items) = value else {
        return Err(ToolsError::AllowedCallers);
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or(ToolsError::AllowedCallers)
        })
        .collect()
}
