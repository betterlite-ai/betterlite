use std::str;

use betterlite_streaming::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, TokenDetails, Usage,
};
use serde::Deserialize;
use serde_json::{Number, Value};
use thiserror::Error;

use crate::gigachat_utils::{
    IdSource, TOOL_CALL_ID_CHARS, TOOL_CALL_ID_PREFIX, TokenUsage, arguments_text, convert_usage,
    generated_id,
};
use crate::openai_base_config::Object;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Chunk(Box<GenericChunk>),
    Done,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the GigaChat stream chunk does not hold the declared fields: {source}")]
    Chunk {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawChunk {
    choices: Vec<RawChoice>,
    usage: Option<TokenUsage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawChoice {
    index: Option<i64>,
    delta: Option<RawDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawDelta {
    content: Option<String>,
    function_call: Option<Value>,
}

pub fn chunk_parser(event: &[u8], ids: &dyn IdSource) -> Result<StreamEvent, StreamError> {
    let text = str::from_utf8(event).unwrap_or_default();
    let payload = text.strip_prefix("data: ").unwrap_or(text);
    if payload.trim() == "[DONE]" {
        return Ok(StreamEvent::Done);
    }
    let raw = match serde_json::from_str::<Value>(payload) {
        Ok(value) => serde_json::from_value::<Object>(value)
            .and_then(|fields| serde_json::from_value::<RawChunk>(Value::Object(fields)))
            .map_err(|source| StreamError::Chunk { source })?,
        Err(_) => RawChunk::default(),
    };
    Ok(StreamEvent::Chunk(Box::new(parse_chunk(raw, ids))))
}

fn parse_chunk(chunk: RawChunk, ids: &dyn IdSource) -> GenericChunk {
    let RawChunk { choices, usage } = chunk;
    let has_choice = !choices.is_empty();
    let choice = choices.into_iter().next().unwrap_or_default();
    let delta = choice.delta.unwrap_or_default();
    let finish = choice.finish_reason;
    let selected = delta
        .function_call
        .as_ref()
        .and_then(Value::as_object)
        .filter(|call| !call.is_empty() && finish.as_deref() == Some("function_call"));
    let (tool_use, finish_reason) = selected.map_or_else(
        || (None, finish.clone().unwrap_or_default()),
        |record| (Some(tool_use_of(record, ids)), String::from("tool_calls")),
    );
    GenericChunk {
        text: delta.content.unwrap_or_default(),
        tool_use,
        is_finished: finish.is_some(),
        finish_reason,
        usage: usage
            .as_ref()
            .filter(|present| has_choice && present.is_present())
            .map(stream_usage),
        index: Some(choice.index.unwrap_or(0)),
        provider_specific_fields: None,
    }
}

fn tool_use_of(call: &Object, ids: &dyn IdSource) -> JsonValue {
    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
    let function = JsonObject::from([
        (String::from("name"), JsonValue::String(name.to_owned())),
        (
            String::from("arguments"),
            JsonValue::String(arguments_text(call.get("arguments"))),
        ),
    ]);
    JsonValue::Object(JsonObject::from([
        (
            String::from("id"),
            JsonValue::String(generated_id(ids, TOOL_CALL_ID_PREFIX, TOOL_CALL_ID_CHARS)),
        ),
        (
            String::from("type"),
            JsonValue::String(String::from("function")),
        ),
        (String::from("function"), JsonValue::Object(function)),
        (
            String::from("index"),
            JsonValue::Number(Number::from(0_i32)),
        ),
    ]))
}

fn stream_usage(usage: &TokenUsage) -> Usage {
    let converted = convert_usage(usage);
    let counter = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    Usage {
        prompt_tokens: counter(converted.prompt_tokens),
        completion_tokens: counter(converted.completion_tokens),
        total_tokens: counter(converted.total_tokens),
        cost: None,
        prompt_tokens_details: converted.cached_tokens.map(|cached| TokenDetails {
            cached_tokens: Some(counter(cached)),
            cache_write_tokens: None,
            cache_creation_tokens: None,
            cache_creation_token_details: None,
            extra: JsonObject::new(),
        }),
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }
}

mod tests;
