use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::records::{CHUNK_OBJECT, ChatChoice, ChatChunk, ChatDelta, ChatToolCall, ChunkFactory};
use super::usage;

const CHUNK_KEYS: [&str; 8] = [
    "id",
    "object",
    "created",
    "model",
    "system_fingerprint",
    "choices",
    "usage",
    "provider_specific_fields",
];
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

// ModelResponseStream(**parsed_chunk): the chunk already speaks chat completions, so only the caller's fields change
pub(super) fn passthrough_chunk(
    fields: &Map<String, Value>,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    let id = match fields.get("id") {
        None | Some(Value::Null) => {
            format!("{CHAT_COMPLETION_ID_PREFIX}{}", (factory.identifier)())
        }
        Some(Value::String(text)) => text.clone(),
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "id" }),
    };
    let created = match fields.get("created") {
        None | Some(Value::Null) => (factory.clock)(),
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or(ResponsesChatStreamError::Field { field: "created" })?,
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "created" }),
    };
    let model = optional_text(fields.get("model"), "model")?;
    let system_fingerprint = optional_text(fields.get("system_fingerprint"), "system_fingerprint")?;
    let choices = choices(fields.get("choices"))?;
    let usage = match fields.get("usage") {
        None => None,
        Some(Value::Null) => Some(Value::Null),
        Some(value) => Some(usage::chat_usage(value)?),
    };
    let provider_specific_fields = match fields.get("provider_specific_fields") {
        None | Some(Value::Null) => None,
        Some(Value::Object(present)) => Some(present.clone()),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "provider_specific_fields",
            });
        }
    };
    let mut extra = Map::new();
    for (key, value) in fields {
        if !CHUNK_KEYS.contains(&key.as_str()) {
            extra.insert(key.clone(), value.clone());
        }
    }
    Ok(ChatChunk {
        id,
        object: CHUNK_OBJECT,
        created,
        model,
        system_fingerprint,
        choices,
        usage,
        provider_specific_fields,
        service_tier: None,
        extra,
    })
}

fn optional_text(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<Option<String>>, ResponsesChatStreamError> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(text)) => Ok(Some(Some(text.clone()))),
        Some(_) => Err(ResponsesChatStreamError::Field { field }),
    }
}

// a choices value that is not a list becomes the one default StreamingChoices, as the constructor writes it
fn choices(value: Option<&Value>) -> Result<Vec<ChatChoice>, ResponsesChatStreamError> {
    let Some(Value::Array(items)) = value else {
        return Ok(vec![ChatChoice {
            index: Some(0),
            delta: ChatDelta::default(),
            finish_reason: None,
            logprobs: None,
            extra: Map::new(),
        }]);
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
    let logprobs = match fields.get("logprobs") {
        None => None,
        Some(value) => Some(value.clone()),
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
        logprobs,
        extra,
    })
}

fn delta(fields: &Map<String, Value>) -> Result<ChatDelta, ResponsesChatStreamError> {
    let content = optional_string(fields.get("content"), "content")?;
    let role = optional_string(fields.get("role"), "role")?;
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
    let audio = match fields.get("audio") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.clone()),
    };
    let reasoning_content = match fields.get("reasoning_content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Null) | None => optional_string(fields.get("reasoning"), "reasoning")?,
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
        audio,
        reasoning_content,
        reasoning_items: None,
        extra,
    })
}

fn function_call(call: &Map<String, Value>) -> Result<Value, ResponsesChatStreamError> {
    let name = optional_string(call.get("name"), "name")?;
    let arguments = optional_string(call.get("arguments"), "arguments")?;
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
        let custom = match fields.get("custom") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.clone()),
        };
        let kind = match fields.get("type") {
            Some(Value::String(text)) => Some(text.clone()),
            None | Some(Value::Null) => Some(String::from(if custom.is_some() {
                "custom"
            } else {
                "function"
            })),
            Some(_) => return Err(ResponsesChatStreamError::Field { field: "type" }),
        };
        let id = optional_string(fields.get("id"), "id")?;
        let function = match fields.get("function") {
            None | Some(Value::Null) => None,
            Some(Value::Object(call)) => Some(function_object(
                optional_string(call.get("name"), "name")?,
                optional_string(call.get("arguments"), "arguments")?,
                call,
            )?),
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

fn function_object(
    name: Option<String>,
    arguments: Option<String>,
    fields: &Map<String, Value>,
) -> Result<Value, ResponsesChatStreamError> {
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
    Ok(Value::Object(function))
}

fn optional_string(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<String>, ResponsesChatStreamError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ResponsesChatStreamError::Field { field }),
    }
}
