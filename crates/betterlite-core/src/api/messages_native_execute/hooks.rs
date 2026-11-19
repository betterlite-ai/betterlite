use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::MessagesExecuteError;

pub(super) fn litellm_params(provider: Option<&str>) -> Json {
    let mut fields: IndexMap<String, Json> = IndexMap::new();
    fields.insert(
        "custom_llm_provider".to_owned(),
        match provider {
            Some(name) => Json::Text(name.to_owned()),
            None => Json::Null,
        },
    );
    Json::Object(fields)
}

// request_kwargs.get("litellm_params", {}).get("custom_llm_provider"), including its AttributeError
pub(super) fn provider_from_hook_map(
    hook_map: &IndexMap<String, Json>,
) -> Result<Option<String>, MessagesExecuteError> {
    let litellm_params = match hook_map.get("litellm_params") {
        None => return Ok(None),
        Some(Json::Object(fields)) => fields,
        Some(Json::Null) => {
            return Err(MessagesExecuteError::MissingAttribute {
                object_type: "NoneType".to_owned(),
            });
        }
        Some(other) => {
            return Err(MessagesExecuteError::MissingAttribute {
                object_type: python_type_name(other).to_owned(),
            });
        }
    };
    match litellm_params.get("custom_llm_provider") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) if name.is_empty() => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(MessagesExecuteError::FieldType {
                field: "litellm_params.custom_llm_provider",
                expected: "a string",
            })
        }
    }
}

fn python_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_i64() || number.is_u64() => "int",
        Json::Number(_) => "float",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

pub(super) fn pop_tools(
    hook_map: &mut IndexMap<String, Json>,
    original: Option<Vec<Json>>,
) -> Result<Option<Vec<Json>>, MessagesExecuteError> {
    match hook_map.shift_remove("tools") {
        None => Ok(original),
        Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => Ok(Some(items)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(MessagesExecuteError::FieldType {
                field: "tools",
                expected: "a list",
            })
        }
    }
}

pub(super) fn pop_stream(
    hook_map: &mut IndexMap<String, Json>,
    original: Option<bool>,
) -> Result<Option<bool>, MessagesExecuteError> {
    match hook_map.shift_remove("stream") {
        None => Ok(original),
        Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(value)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(MessagesExecuteError::FieldType {
                field: "stream",
                expected: "a boolean",
            })
        }
    }
}

// An explicit null in the hook map is the named argument's None, not a kept original
pub(super) fn pop_named(
    hook_map: &mut IndexMap<String, Json>,
    name: &str,
    original: Option<Json>,
) -> Option<Json> {
    match hook_map.shift_remove(name) {
        None => original,
        Some(Json::Null) => None,
        Some(value) => Some(value),
    }
}
