use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use serde_json::{Map, Value};

use super::error::ResponsesChatStreamError;
use super::fields::Fields;
use super::records::{CHUNK_OBJECT, ChatChunk, ChunkFactory};
use super::usage;

mod parts;
mod tools;

const PASSTHROUGH_KEYS: [&str; 9] = [
    "id",
    "object",
    "created",
    "model",
    "system_fingerprint",
    "choices",
    "usage",
    "provider_specific_fields",
    "service_tier",
];

// ModelResponseStream(**parsed_chunk): the event already speaks chat completions, so only defaults and coercions apply
pub(super) fn passthrough_chunk(
    fields: &Fields,
    factory: ChunkFactory,
) -> Result<ChatChunk, ResponsesChatStreamError> {
    let id = chunk_id(fields, factory)?;
    let created = chunk_created(fields, factory)?;
    let model = optional_text(fields.get("model"), "model")?;
    let system_fingerprint = optional_text(fields.get("system_fingerprint"), "system_fingerprint")?;
    let service_tier = optional_text(fields.get("service_tier"), "service_tier")?;
    let choices = parts::choices(fields.get("choices"))?;
    Ok(ChatChunk {
        id,
        object: CHUNK_OBJECT,
        created,
        model,
        system_fingerprint,
        choices,
        usage: chunk_usage(fields)?,
        provider_specific_fields: provider_fields(fields)?,
        service_tier,
        extra: extra_fields(fields),
    })
}

fn chunk_id(fields: &Fields, factory: ChunkFactory) -> Result<String, ResponsesChatStreamError> {
    match fields.get("id") {
        None | Some(Value::Null) => Ok(format!(
            "{CHAT_COMPLETION_ID_PREFIX}{}",
            (factory.identifier)()
        )),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "id" }),
    }
}

fn chunk_created(fields: &Fields, factory: ChunkFactory) -> Result<i64, ResponsesChatStreamError> {
    match fields.get("created") {
        None | Some(Value::Null) => Ok((factory.clock)()),
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or(ResponsesChatStreamError::Field { field: "created" }),
        Some(_) => Err(ResponsesChatStreamError::Field { field: "created" }),
    }
}

fn chunk_usage(fields: &Fields) -> Result<Option<Value>, ResponsesChatStreamError> {
    match fields.get("usage") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(Value::Null)),
        Some(value) => Ok(Some(usage::chat_usage(value)?)),
    }
}

fn provider_fields(
    fields: &Fields,
) -> Result<Option<Map<String, Value>>, ResponsesChatStreamError> {
    match fields.get("provider_specific_fields") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(present)) => Ok(Some(present.clone())),
        Some(_) => Err(ResponsesChatStreamError::Field {
            field: "provider_specific_fields",
        }),
    }
}

fn extra_fields(fields: &Fields) -> Map<String, Value> {
    let mut extra = Map::new();
    for (key, value) in fields {
        if !PASSTHROUGH_KEYS.contains(&key.as_str()) {
            extra.insert(key.clone(), value.clone());
        }
    }
    extra
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
