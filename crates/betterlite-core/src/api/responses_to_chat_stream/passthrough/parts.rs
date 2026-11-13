use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;
use serde_json::{Map, Value};

use super::super::error::ResponsesChatStreamError;
use super::super::fields::Fields;
use super::super::records::{ChatChoice, ChatDelta, ChatToolCall};
use super::optional_text;

const CHOICE_KEYS: [&str; 4] = ["index", "delta", "finish_reason", "logprobs"];
const DELTA_KEYS: [&str; 7] = [
    "content",
    "role",
    "function_call",
    "tool_calls",
    "audio",
    "reasoning",
    "reasoning_content",
];
const TOOL_CALL_KEYS: [&str; 5] = ["id", "index", "type", "function", "custom"];
const FUNCTION_KEYS: [&str; 2] = ["name", "arguments"];

pub(super) fn choices(value: Option<&Value>) -> Result<Vec<ChatChoice>, ResponsesChatStreamError> {
    let Some(Value::Array(items)) = value else {
        return Ok(super::default_choice());
    };
    let mut choices = Vec::with_capacity(items.len());
    for item in items {
        choices.push(choice(item)?);
    }
    Ok(choices)
}

fn choice(value: &Value) -> Result<ChatChoice, ResponsesChatStreamError> {
    let Value::Object(fields) = value else {
        return Err(ResponsesChatStreamError::Field { field: "choices" });
    };
    let index = match fields.get("index") {
        None => Some(0),
        Some(Value::Null) => None,
        Some(Value::Number(number)) => Some(
            number
                .as_i64()
                .ok_or(ResponsesChatStreamError::Field { field: "index" })?,
        ),
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "index" }),
    };
    let finish_reason = match fields.get("finish_reason") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(Value::String(text)) => Some(mapped_finish_reason(text).to_owned()),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "finish_reason",
            });
        }
    };
    let delta = match fields.get("delta") {
        None | Some(Value::Null) => ChatDelta::default(),
        Some(Value::Object(delta_fields)) => delta(delta_fields)?,
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "delta" }),
    };
    let mut extra = Map::new();
    for (key, value) in fields {
        if !CHOICE_KEYS.contains(&key.as_str()) {
            extra.insert(key.clone(), value.clone());
        }
    }
    Ok(ChatChoice {
        index,
        delta,
        finish_reason,
        logprobs: fields.get("logprobs").cloned(),
        extra,
    })
}

fn delta(fields: &Fields) -> Result<ChatDelta, ResponsesChatStreamError> {
    let content = optional_text(fields.get("content"), "content")?.flatten();
    let role = optional_text(fields.get("role"), "role")?.flatten();
    let function_call = match fields.get("function_call") {
        None | Some(Value::Null) => None,
        Some(Value::Object(call)) => Some(function_call(call)?),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "function_call",
            });
        }
    };
    let tool_calls = match fields.get("tool_calls") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => Some(tool_calls(items)?),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "tool_calls",
            });
        }
    };
    let reasoning = match fields.get("reasoning_content") {
        Some(Value::Null) | None => optional_text(fields.get("reasoning"), "reasoning")?.flatten(),
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "reasoning_content",
            });
        }
    };
    let mut extra = Map::new();
    for (key, value) in fields {
        if DELTA_KEYS.contains(&key.as_str()) {
            continue;
        }
        if key.as_str() == "annotations" && value.is_null() {
            continue;
        }
        if key.as_str() == "images" && matches!(value, Value::Array(items) if items.is_empty()) {
            continue;
        }
        extra.insert(key.clone(), value.clone());
    }
    Ok(ChatDelta {
        content,
        role,
        function_call,
        tool_calls,
        audio: fields
            .get("audio")
            .filter(|value| !value.is_null())
            .cloned(),
        reasoning_content: reasoning,
        reasoning_items: None,
        extra,
    })
}

fn function_call(call: &Fields) -> Result<Value, ResponsesChatStreamError> {
    let name = optional_text(call.get("name"), "name")?.flatten();
    let arguments = optional_text(call.get("arguments"), "arguments")?.flatten();
    Ok(function_object(name, arguments, call))
}

fn tool_calls(items: &[Value]) -> Result<Vec<ChatToolCall>, ResponsesChatStreamError> {
    let mut calls = Vec::with_capacity(items.len());
    let mut next_index = 0_i64;
    for item in items {
        let Value::Object(fields) = item else {
            return Err(ResponsesChatStreamError::Field {
                field: "tool_calls",
            });
        };
        let index = match fields.get("index") {
            None | Some(Value::Null) => {
                let assigned = next_index;
                next_index = next_index.saturating_add(1);
                Some(assigned)
            }
            Some(Value::Number(number)) => Some(
                number
                    .as_i64()
                    .ok_or(ResponsesChatStreamError::Field { field: "index" })?,
            ),
            Some(_) => return Err(ResponsesChatStreamError::Field { field: "index" }),
        };
        let custom = fields
            .get("custom")
            .filter(|value| !value.is_null())
            .cloned();
        let kind = match fields.get("type") {
            Some(Value::String(text)) => Some(text.clone()),
            None | Some(Value::Null) => Some(String::from(if custom.is_some() {
                "custom"
            } else {
                "function"
            })),
            Some(_) => return Err(ResponsesChatStreamError::Field { field: "type" }),
        };
        let id = optional_text(fields.get("id"), "id")?.flatten();
        let function = match fields.get("function") {
            None | Some(Value::Null) => None,
            Some(Value::Object(call)) => Some(function_object(
                optional_text(call.get("name"), "name")?.flatten(),
                optional_text(call.get("arguments"), "arguments")?.flatten(),
                call,
            )),
            Some(_) => return Err(ResponsesChatStreamError::Field { field: "function" }),
        };
        let mut extra = Map::new();
        for (key, value) in fields {
            if !TOOL_CALL_KEYS.contains(&key.as_str()) {
                extra.insert(key.clone(), value.clone());
            }
        }
        calls.push(ChatToolCall {
            id,
            index,
            kind,
            function,
            custom,
            provider_specific_fields: None,
            extra,
        });
    }
    Ok(calls)
}

fn function_object(name: Option<String>, arguments: Option<String>, fields: &Fields) -> Value {
    let mut function = Map::new();
    function.insert("name".to_owned(), name.map_or(Value::Null, Value::String));
    function.insert(
        "arguments".to_owned(),
        arguments.map_or(Value::Null, Value::String),
    );
    for (key, value) in fields {
        if !FUNCTION_KEYS.contains(&key.as_str()) {
            function.insert(key.clone(), value.clone());
        }
    }
    Value::Object(function)
}
