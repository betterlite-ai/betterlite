use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::NamespaceToolsError;

pub type Fields = IndexMap<String, Json>;

// LiteLLM tests the extras of a top-level function tool and a namespace's strict flag with `or` and bool(), and the task requires the same
#[must_use]
pub fn has_value(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => false,
        Json::Bool(true) => true,
        Json::Number(number) => number.as_f64() != Some(0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
    }
}

// `str(tool.get(field) or "")`: absent and null read as empty, any other type is the caller's mistake
pub fn text_field<'fields>(
    fields: &'fields Fields,
    field: &'static str,
) -> Result<&'fields str, NamespaceToolsError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(""),
        Some(Json::Text(text)) => Ok(text),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(NamespaceToolsError::WrongType {
                field,
                expected: "a string",
            })
        }
    }
}

// a tool whose type is missing or not text matches no type branch, so it reads as ""
#[must_use]
pub fn tool_type(tool: &Fields) -> &str {
    match tool.get("type") {
        Some(Json::Text(text)) => text,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            ""
        }
    }
}

pub fn tool_objects(tools: &[Json]) -> Result<Vec<&Fields>, NamespaceToolsError> {
    tools
        .iter()
        .enumerate()
        .map(|(position, tool)| match tool {
            Json::Object(fields) => Ok(fields),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Err(NamespaceToolsError::ToolNotObject { position })
            }
        })
        .collect()
}

// Python accepts any sequence except str and bytes; JSON has only lists
#[must_use]
pub fn member_list(namespace: &Fields) -> Option<&[Json]> {
    match namespace.get("tools") {
        Some(Json::List(members)) => Some(members),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            None
        }
    }
}
