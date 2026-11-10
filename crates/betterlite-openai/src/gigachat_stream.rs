use std::num::TryFromIntError;
use std::str;

use betterlite_streaming::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, TokenDetails, Usage,
};
use serde::Deserialize;
use serde_json::Number;
use serde_json::Value;
use thiserror::Error;

use crate::gigachat_utils::{
    IdSource, TOOL_CALL_ID_CHARS, TOOL_CALL_ID_PREFIX, TokenUsage, UsageError, arguments_text,
    convert_usage,
};
use crate::openai_base_config::Object;

// LiteLLM's generic streaming chunk for GigaChat: a chunk the shared chunk decoder reads as is
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
    #[error("the GigaChat stream counter {value} does not fit the chunk usage")]
    Counter {
        value: u64,
        #[source]
        source: TryFromIntError,
    },
    #[error(transparent)]
    Usage(#[from] UsageError),
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
    // LiteLLM reads a line that is not JSON as an empty chunk and a JSON line of the wrong shape as a failure
    let raw = match serde_json::from_str::<Value>(payload) {
        Ok(value) => serde_json::from_value::<RawChunk>(value)
            .map_err(|source| StreamError::Chunk { source })?,
        Err(_) => RawChunk::default(),
    };
    Ok(StreamEvent::Chunk(Box::new(parse_chunk(raw, ids)?)))
}

fn parse_chunk(chunk: RawChunk, ids: &dyn IdSource) -> Result<GenericChunk, StreamError> {
    let RawChunk { choices, usage } = chunk;
    let Some(choice) = choices.into_iter().next() else {
        return Ok(GenericChunk {
            text: String::new(),
            tool_use: None,
            is_finished: false,
            finish_reason: String::new(),
            usage: None,
            index: Some(0),
            provider_specific_fields: None,
        });
    };
    let delta = choice.delta.unwrap_or_default();
    let finish = choice.finish_reason;
    let call = chunk_call(&delta, finish.as_deref(), ids);
    let text = delta.content.unwrap_or_default();
    let chunk_usage = usage
        .as_ref()
        .filter(|present| present.is_present())
        .map(stream_usage)
        .transpose()?;
    Ok(GenericChunk {
        text,
        tool_use: call.tool_use,
        is_finished: finish.is_some(),
        finish_reason: call.finish_reason,
        usage: chunk_usage,
        index: Some(choice.index.unwrap_or(0)),
        provider_specific_fields: None,
    })
}

struct StreamCall {
    tool_use: Option<JsonValue>,
    finish_reason: String,
}

fn chunk_call(delta: &RawDelta, finish: Option<&str>, ids: &dyn IdSource) -> StreamCall {
    let selected = delta
        .function_call
        .as_ref()
        .and_then(Value::as_object)
        .filter(|call| !call.is_empty())
        .filter(|_| finish == Some("function_call"));
    selected.map_or_else(
        || StreamCall {
            tool_use: None,
            finish_reason: finish.unwrap_or_default().to_owned(),
        },
        |record| StreamCall {
            tool_use: Some(JsonValue::Object(tool_use_of(record, ids))),
            finish_reason: String::from("tool_calls"),
        },
    )
}

fn tool_use_of(call: &Object, ids: &dyn IdSource) -> JsonObject {
    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
    let hex: String = ids
        .new_uuid()
        .simple()
        .to_string()
        .chars()
        .take(TOOL_CALL_ID_CHARS)
        .collect();
    let mut function = JsonObject::new();
    function.insert(String::from("name"), JsonValue::String(name.to_owned()));
    function.insert(
        String::from("arguments"),
        JsonValue::String(arguments_text(call.get("arguments"))),
    );
    let mut tool = JsonObject::new();
    tool.insert(
        String::from("id"),
        JsonValue::String(format!("{TOOL_CALL_ID_PREFIX}{hex}")),
    );
    tool.insert(
        String::from("type"),
        JsonValue::String(String::from("function")),
    );
    tool.insert(String::from("function"), JsonValue::Object(function));
    tool.insert(
        String::from("index"),
        JsonValue::Number(Number::from(0_i64)),
    );
    tool
}

fn stream_usage(usage: &TokenUsage) -> Result<Usage, StreamError> {
    let converted = convert_usage(usage)?;
    let counter =
        |value: u64| i64::try_from(value).map_err(|source| StreamError::Counter { value, source });
    let prompt_tokens = counter(converted.prompt_tokens)?;
    let completion_tokens = counter(converted.completion_tokens)?;
    let total_tokens = counter(converted.total_tokens)?;
    let cached_tokens = converted.cached_tokens.map(counter).transpose()?;
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cost: None,
        prompt_tokens_details: cached_tokens.map(|cached| TokenDetails {
            cached_tokens: Some(cached),
            cache_write_tokens: None,
            cache_creation_tokens: None,
            cache_creation_token_details: None,
            extra: JsonObject::new(),
        }),
        completion_tokens_details: None,
        extra: JsonObject::new(),
    })
}

mod tests;
