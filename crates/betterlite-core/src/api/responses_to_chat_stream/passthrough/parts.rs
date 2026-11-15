use betterlite_streaming::stream_chunk_shaping::tokens::mapped_finish_reason;
use serde_json::{Map, Value};

use super::super::error::ResponsesChatStreamError;
use super::super::fields::{Fields, field};
use super::super::records::{ChatChoice, ChatDelta};
use super::optional_text;
use super::tools;

const CHOICE_KEYS: [&str; 5] = ["index", "delta", "finish_reason", "logprobs", "message"];
const DELTA_KEYS: [&str; 6] = [
    "content",
    "role",
    "function_call",
    "tool_calls",
    "audio",
    "reasoning_content",
];
const FUNCTION_KEYS: [&str; 2] = ["name", "arguments"];

pub(super) fn choices(value: Option<&Value>) -> Result<Vec<ChatChoice>, ResponsesChatStreamError> {
    let Some(Value::Array(items)) = value else {
        // not a list: the one default StreamingChoices the constructor writes
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
    // StreamingChoices defaults the index to 0, while an explicit null stays null in the dump
    let index = if fields.contains_key("index") {
        field::<i64>(fields, "index")?
    } else {
        Some(0)
    };
    let delta_value = match fields.get("delta") {
        None | Some(Value::Null) => ChatDelta::default(),
        Some(Value::Object(delta_fields)) => delta(delta_fields)?,
        Some(_) => return Err(ResponsesChatStreamError::Field { field: "delta" }),
    };
    let finish_reason = match field::<&str>(fields, "finish_reason")? {
        None | Some("") => None,
        Some(text) => Some(mapped_finish_reason(text).to_owned()),
    };
    Ok(ChatChoice {
        index,
        delta: delta_value,
        finish_reason,
        logprobs: choice_logprobs(fields.get("logprobs")),
        extra: choice_extra(fields),
    })
}

// ChoiceLogprobs normalizes a null top_logprobs on every content token to an empty list
fn choice_logprobs(value: Option<&Value>) -> Option<Value> {
    let Some(Value::Object(logprobs)) = value else {
        return value.cloned();
    };
    let mut normalized = logprobs.clone();
    if let Some(Value::Array(tokens)) = normalized.get_mut("content") {
        for token in tokens {
            let Some(fields) = token.as_object_mut() else {
                continue;
            };
            if fields.get("top_logprobs").is_some_and(Value::is_null) {
                fields.insert("top_logprobs".to_owned(), Value::Array(Vec::new()));
            }
        }
    }
    Some(Value::Object(normalized))
}

// StreamingChoices sets enhancements only when it holds a value
fn choice_extra(fields: &Fields) -> Map<String, Value> {
    let mut extra = fields.clone();
    for key in CHOICE_KEYS {
        extra.remove(key);
    }
    if extra.get("enhancements").is_some_and(Value::is_null) {
        extra.remove("enhancements");
    }
    extra
}

fn delta(fields: &Fields) -> Result<ChatDelta, ResponsesChatStreamError> {
    let function_call = match fields.get("function_call") {
        None | Some(Value::Null) => None,
        Some(Value::Object(call)) => Some(function_call_value(call)?),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "function_call",
            });
        }
    };
    let tool_calls = match fields.get("tool_calls") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => Some(tools::tool_calls(items)?),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "tool_calls",
            });
        }
    };
    // reasoning_content wins, and an unset one falls back to the providers' reasoning key
    let reasoning_content = match fields.get("reasoning_content") {
        Some(Value::Null) | None => optional_text(fields.get("reasoning"), "reasoning")?.flatten(),
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => {
            return Err(ResponsesChatStreamError::Field {
                field: "reasoning_content",
            });
        }
    };
    Ok(ChatDelta {
        content: optional_text(fields.get("content"), "content")?.flatten(),
        role: optional_text(fields.get("role"), "role")?.flatten(),
        function_call,
        tool_calls,
        audio: fields
            .get("audio")
            .filter(|value| !value.is_null())
            .cloned(),
        reasoning_content,
        reasoning_items: None,
        extra: delta_extra(fields),
    })
}

fn delta_extra(fields: &Fields) -> Map<String, Value> {
    let reasoning_consumed = fields.get("reasoning_content").is_none_or(Value::is_null);
    let mut extra = Map::new();
    for (key, value) in fields {
        if DELTA_KEYS.contains(&key.as_str()) {
            continue;
        }
        if key.as_str() == "reasoning" && reasoning_consumed {
            continue;
        }
        if value.is_null()
            && matches!(
                key.as_str(),
                "annotations" | "images" | "reasoning_items" | "thinking_blocks"
            )
        {
            continue;
        }
        if key.as_str() == "images" && matches!(value, Value::Array(items) if items.is_empty()) {
            continue;
        }
        extra.insert(key.clone(), value.clone());
    }
    extra
}

// FunctionCall requires a string arguments value and keeps unknown keys as extra attributes
fn function_call_value(call: &Fields) -> Result<Value, ResponsesChatStreamError> {
    let arguments = optional_text(call.get("arguments"), "arguments")?
        .flatten()
        .ok_or(ResponsesChatStreamError::Field { field: "arguments" })?;
    let mut function = Map::new();
    function.insert("arguments".to_owned(), Value::String(arguments));
    if let Some(name) = optional_text(call.get("name"), "name")? {
        function.insert("name".to_owned(), name.map_or(Value::Null, Value::String));
    }
    for (key, value) in call {
        if !FUNCTION_KEYS.contains(&key.as_str()) {
            function.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::Object(function))
}
