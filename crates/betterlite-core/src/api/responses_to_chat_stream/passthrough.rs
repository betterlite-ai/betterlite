use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::fields::Fields;
use super::records::{CHUNK_OBJECT, ChatChoice, ChatChunk, ChatDelta, ChunkFactory};
use super::usage;

mod parts;

// ModelResponseStream(**parsed_chunk): the event already speaks chat completions, so only defaults and coercions apply
pub(super) fn passthrough_chunk(
    fields: &Fields,
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
    let choices = parts::choices(fields.get("choices"))?;
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
        if !matches!(
            key.as_str(),
            "id" | "object"
                | "created"
                | "model"
                | "system_fingerprint"
                | "choices"
                | "usage"
                | "provider_specific_fields"
        ) {
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

// an absent key, an explicit null and a string are the three states ModelResponseStream dumps differently
pub(super) fn optional_text(
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

// a choices value that is not a list becomes the one default StreamingChoices the constructor writes
fn default_choice() -> Vec<ChatChoice> {
    vec![ChatChoice {
        index: Some(0),
        delta: ChatDelta::default(),
        finish_reason: None,
        logprobs: None,
        extra: Map::new(),
    }]
}
