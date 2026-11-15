use serde_json::{Map, Value};

use super::super::error::ResponsesChatStreamError;
use super::super::fields::Fields;
use super::super::records::ChatToolCall;
use super::optional_text;

const TOOL_CALL_KEYS: [&str; 5] = ["id", "index", "type", "function", "custom"];

pub(super) fn tool_calls(items: &[Value]) -> Result<Vec<ChatToolCall>, ResponsesChatStreamError> {
    let mut calls = Vec::with_capacity(items.len());
    let mut next_index = 0_i64;
    for item in items {
        let Value::Object(fields) = item else {
            return Err(ResponsesChatStreamError::Field {
                field: "tool_calls",
            });
        };
        let index = call_index(fields, &mut next_index)?;
        calls.push(call_shape(fields, index)?);
    }
    Ok(calls)
}

// Delta.__init__ fills a missing index with the next position in the list
fn call_index(
    fields: &Fields,
    next_index: &mut i64,
) -> Result<Option<i64>, ResponsesChatStreamError> {
    match fields.get("index") {
        None | Some(Value::Null) => {
            let assigned = *next_index;
            *next_index = next_index.saturating_add(1);
            Ok(Some(assigned))
        }
        Some(Value::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(ResponsesChatStreamError::Field { field: "index" }),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "index" }),
    }
}

// ChatCompletionDeltaCustomToolCall: a nonnull custom payload picks the custom shape
fn call_shape(
    fields: &Fields,
    index: Option<i64>,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    let custom = fields.get("custom").filter(|value| !value.is_null());
    if let Some(custom) = custom {
        return custom_tool_call(fields, custom, index);
    }
    function_tool_call(fields, index)
}

fn custom_tool_call(
    fields: &Fields,
    custom: &Value,
    index: Option<i64>,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    let Some(payload) = custom.as_object() else {
        return Err(ResponsesChatStreamError::Field { field: "custom" });
    };
    for (key, field) in [("name", "custom.name"), ("input", "custom.input")] {
        if payload
            .get(key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(ResponsesChatStreamError::Field { field });
        }
    }
    Ok(ChatToolCall {
        id: optional_text(fields.get("id"), "id")?,
        index,
        kind: custom_kind(fields)?,
        function: None,
        custom: Some(custom.clone()),
        provider_specific_fields: None,
        extra: tool_call_extra(fields),
    })
}

fn custom_kind(fields: &Fields) -> Result<Option<Option<String>>, ResponsesChatStreamError> {
    match fields.get("type") {
        Some(Value::String(text)) => Ok(Some(Some(text.clone()))),
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "type" }),
    }
}

fn function_tool_call(
    fields: &Fields,
    index: Option<i64>,
) -> Result<ChatToolCall, ResponsesChatStreamError> {
    let function = match fields.get("function") {
        Some(Value::Object(call)) => tool_function(call)?,
        _ => {
            return Err(ResponsesChatStreamError::Field { field: "function" });
        }
    };
    Ok(ChatToolCall {
        id: optional_text(fields.get("id"), "id")?,
        index,
        kind: function_kind(fields)?,
        function: Some(function),
        custom: fields.get("custom").cloned(),
        provider_specific_fields: None,
        extra: tool_call_extra(fields),
    })
}

fn function_kind(fields: &Fields) -> Result<Option<Option<String>>, ResponsesChatStreamError> {
    match fields.get("type") {
        Some(Value::String(text)) => Ok(Some(Some(text.clone()))),
        None | Some(Value::Null) => Ok(Some(Some("function".to_owned()))),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "type" }),
    }
}

fn tool_call_extra(fields: &Fields) -> Map<String, Value> {
    let mut extra = Map::new();
    for (key, value) in fields {
        if !TOOL_CALL_KEYS.contains(&key.as_str()) {
            extra.insert(key.clone(), value.clone());
        }
    }
    extra
}

// Function.__init__ sets name and arguments only, with dict arguments carried as a JSON string
fn tool_function(call: &Fields) -> Result<Value, ResponsesChatStreamError> {
    let arguments = match call.get("arguments") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(fields)) => json_text(fields)?,
        None | Some(Value::Null) => match call.get("parameters") {
            Some(Value::Object(fields)) => json_text(fields)?,
            None | Some(_) => String::new(),
        },
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "arguments" }),
    };
    let mut function = Map::new();
    function.insert(
        "name".to_owned(),
        optional_text(call.get("name"), "name")?
            .flatten()
            .map_or(Value::Null, Value::String),
    );
    function.insert("arguments".to_owned(), Value::String(arguments));
    Ok(Value::Object(function))
}

fn json_text(fields: &Map<String, Value>) -> Result<String, ResponsesChatStreamError> {
    serde_json::to_string(fields).map_err(|source| ResponsesChatStreamError::Json {
        message: source.to_string(),
        source,
    })
}
