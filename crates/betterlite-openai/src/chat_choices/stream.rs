use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::chat_choices::wire::{StreamChoice, StreamDelta, StreamReply, StreamUsage};
use crate::text_completion_policy::reply::mapped_finish_reason;

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";

// get_model_response_iterator's handler; json_mode travels with the iterator as BaseModelResponseIterator keeps it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingHandler {
    pub json_mode: bool,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the stream chunk is not an object")]
    ChunkNotObject,
    #[error("the stream chunk's choices are not a list")]
    ChoicesNotList,
    #[error("a stream choice is not an object")]
    ChoiceNotObject,
    #[error("a stream choice's delta is not an object")]
    DeltaNotObject,
    #[error("a stream choice's index is not an integer")]
    IndexNotInteger,
    #[error("a stream choice's finish_reason is not text")]
    FinishReasonNotText,
    #[error("a stream chunk's id is not text")]
    IdNotText,
    #[error("a stream chunk's created is not an integer")]
    CreatedNotInteger,
    #[error("a stream chunk's model is not text")]
    ModelNotText,
    #[error("a stream chunk's usage is not an object")]
    UsageNotObject,
    #[error("the provider reported an error inside the stream: {message}")]
    Provider { message: String, status: u16 },
}

pub fn map_reasoning_to_reasoning_content(choices: &mut [Value]) -> Result<(), StreamError> {
    for choice in choices {
        let fields = choice.as_object_mut().ok_or(StreamError::ChoiceNotObject)?;
        let Some(delta) = fields.get_mut("delta") else {
            continue;
        };
        let delta_fields = delta.as_object_mut().ok_or(StreamError::DeltaNotObject)?;
        if let Some(reasoning) = delta_fields.remove("reasoning") {
            delta_fields.insert(String::from("reasoning_content"), reasoning);
        }
    }
    Ok(())
}

#[must_use]
pub fn extract_error_from_chunk(chunk: &Value) -> Option<(String, u16)> {
    let error = chunk.get("error")?;
    if is_blank(error) {
        return None;
    }
    if !error.is_object() {
        return Some((error.to_string(), 500));
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
    let fields = chunk.as_object().ok_or(StreamError::ChunkNotObject)?;
    if let Some((message, status)) = extract_error_from_chunk(chunk) {
        return Err(StreamError::Provider { message, status });
    }
    let mut choices = match fields.get("choices") {
        None => Vec::new(),
        Some(Value::Array(choices)) => choices.clone(),
        Some(_) => return Err(StreamError::ChoicesNotList),
    };
    map_reasoning_to_reasoning_content(&mut choices)?;
    Ok(StreamReply {
        id: chunk_id(fields.get("id"), identifier)?,
        object: CHUNK_OBJECT,
        created: chunk_created(fields.get("created"), clock)?,
        model: chunk_model(fields.get("model"))?,
        choices: choices
            .iter()
            .map(stream_choice)
            .collect::<Result<Vec<_>, _>>()?,
        usage: chunk_usage(fields.get("usage"))?,
        service_tier: chunk_service_tier(fields.get("service_tier")),
    })
}

fn chunk_id(id: Option<&Value>, identifier: fn() -> Uuid) -> Result<String, StreamError> {
    match id {
        None | Some(Value::Null) => Ok(format!("chatcmpl-{}", identifier())),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(StreamError::IdNotText),
    }
}

fn chunk_created(created: Option<&Value>, clock: fn() -> i64) -> Result<i64, StreamError> {
    match created {
        None | Some(Value::Null) => Ok(clock()),
        Some(Value::Number(number)) => number.as_i64().ok_or(StreamError::CreatedNotInteger),
        Some(_) => Err(StreamError::CreatedNotInteger),
    }
}

fn chunk_model(model: Option<&Value>) -> Result<Option<String>, StreamError> {
    match model {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(StreamError::ModelNotText),
    }
}

fn chunk_usage(usage: Option<&Value>) -> Result<Option<StreamUsage>, StreamError> {
    match usage {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_error| StreamError::UsageNotObject),
    }
}

fn chunk_service_tier(tier: Option<&Value>) -> Option<String> {
    match tier {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn stream_choice(choice: &Value) -> Result<StreamChoice, StreamError> {
    let fields = choice.as_object().ok_or(StreamError::ChoiceNotObject)?;
    Ok(StreamChoice {
        index: choice_index(fields.get("index"))?,
        finish_reason: choice_finish(fields.get("finish_reason"))?,
        delta: choice_delta(fields.get("delta"))?,
        logprobs: fields.get("logprobs").cloned(),
    })
}

fn choice_index(index: Option<&Value>) -> Result<i64, StreamError> {
    match index {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(number)) => number.as_i64().ok_or(StreamError::IndexNotInteger),
        Some(_) => Err(StreamError::IndexNotInteger),
    }
}

// StreamingChoices maps a nonempty finish_reason and leaves an absent or empty one null
fn choice_finish(finish_reason: Option<&Value>) -> Result<Option<String>, StreamError> {
    match finish_reason {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(mapped_finish_reason(text).to_owned())),
        Some(_) => Err(StreamError::FinishReasonNotText),
    }
}

fn choice_delta(delta: Option<&Value>) -> Result<StreamDelta, StreamError> {
    delta.map_or_else(
        || Ok(StreamDelta::default()),
        |value| serde_json::from_value(value.clone()).map_err(|_error| StreamError::DeltaNotObject),
    )
}
