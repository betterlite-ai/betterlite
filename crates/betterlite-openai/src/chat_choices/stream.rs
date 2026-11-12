use std::mem;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::chat_choices::wire::{
    ProviderField, StreamChoice, StreamDelta, StreamReply, StreamUsage,
};
use crate::text_completion_policy::reply::mapped_finish_reason;

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";

// get_model_response_iterator's handler; json_mode travels with the iterator as BaseModelResponseIterator keeps it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingHandler {
    pub json_mode: bool,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the stream chunk does not have the shape of a chat completion chunk")]
    Chunk(#[source] serde_json::Error),
    #[error("a stream tool call carries neither a function nor a custom payload")]
    ToolCallPayload,
    #[error("the provider reported an error inside the stream: {message}")]
    Provider { message: String, status: u16 },
}

// the error key is read before anything else and LiteLLM accepts any JSON type there
#[derive(Deserialize)]
struct ErrorProbe {
    #[serde(default)]
    error: Option<Value>,
}

// service_tier stays open because anything but a nonempty string is dropped
#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<StreamUsage>,
    #[serde(default)]
    service_tier: Option<Value>,
}

#[must_use]
pub fn extract_error_from_chunk(error: &Value) -> Option<(String, u16)> {
    if is_blank(error) {
        return None;
    }
    if !error.is_object() {
        return Some((
            error
                .as_str()
                .map_or_else(|| error.to_string(), str::to_owned),
            500,
        ));
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map_or_else(|| error.to_string(), str::to_owned);
    let status = error
        .get("code")
        .and_then(Value::as_i64)
        .filter(|code| (400..600).contains(code))
        .and_then(|code| u16::try_from(code).ok())
        .unwrap_or(500);
    Some((message, status))
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        Value::Number(number) => number.as_f64() == Some(0.0_f64),
        Value::Bool(true) => false,
    }
}

pub fn chunk_parser(
    chunk: &Value,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<StreamReply, StreamError> {
    let probe = ErrorProbe::deserialize(chunk).map_err(StreamError::Chunk)?;
    if let Some((message, status)) = probe.error.as_ref().and_then(extract_error_from_chunk) {
        return Err(StreamError::Provider { message, status });
    }
    let parsed = Chunk::deserialize(chunk).map_err(StreamError::Chunk)?;
    let mut choices = parsed.choices;
    for choice in &mut choices {
        rename_reasoning(&mut choice.delta);
        number_tool_calls(&mut choice.delta)?;
        choice.finish_reason = choice
            .finish_reason
            .take()
            .filter(|reason| !reason.is_empty())
            .map(|reason| mapped_finish_reason(&reason).to_owned());
    }
    Ok(StreamReply {
        id: parsed
            .id
            .unwrap_or_else(|| format!("chatcmpl-{}", identifier())),
        object: CHUNK_OBJECT,
        created: parsed.created.unwrap_or_else(clock),
        model: parsed.model,
        choices,
        usage: parsed.usage,
        service_tier: match parsed.service_tier {
            Some(Value::String(tier)) if !tier.is_empty() => Some(tier),
            _ => None,
        },
    })
}

// a present reasoning key replaces reasoning_content, even when it is null
pub fn rename_reasoning(delta: &mut StreamDelta) {
    match mem::take(&mut delta.reasoning) {
        ProviderField::Absent => {}
        ProviderField::Null => delta.reasoning_content = None,
        ProviderField::Text(text) => delta.reasoning_content = Some(text),
    }
}

fn number_tool_calls(delta: &mut StreamDelta) -> Result<(), StreamError> {
    let mut numbers = 0_i64..;
    for call in delta.tool_calls.iter_mut().flatten() {
        call.index = call.index.or_else(|| numbers.next());
        if call.custom.is_some() || call.kind.as_deref() == Some("custom") {
            if call.custom.is_none() {
                return Err(StreamError::ToolCallPayload);
            }
            continue;
        }
        if call.function.is_none() {
            return Err(StreamError::ToolCallPayload);
        }
        call.kind.get_or_insert_with(|| String::from("function"));
    }
    Ok(())
}
