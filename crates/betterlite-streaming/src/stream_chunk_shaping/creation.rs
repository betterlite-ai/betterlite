use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Number, from_value, to_value};
use uuid::Uuid;

use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::error::StreamShapeError;
use crate::stream_chunk_shaping::records::{
    ChunkArguments, JsonObject, JsonValue, ParsedChunkHiddenParams, StreamChoice, StreamChunk,
};

pub struct StreamInstant {
    pub seconds: i64,
    pub timestamp: f64,
}

pub trait StreamIdentity: Send + Sync {
    fn new_id(&self) -> String;
    fn instant(&self) -> StreamInstant;
}

pub struct SystemStreamIdentity;

impl StreamIdentity for SystemStreamIdentity {
    fn new_id(&self) -> String {
        Uuid::new_v4().to_string()
    }

    fn instant(&self) -> StreamInstant {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(
            StreamInstant {
                seconds: 0,
                timestamp: 0.0,
            },
            |elapsed| StreamInstant {
                seconds: i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
                timestamp: elapsed.as_secs_f64(),
            },
        )
    }
}

pub fn model_response_creator(
    shaper: &mut ChunkShaper,
    chunk: Option<&JsonObject>,
    hidden_params: Option<&JsonObject>,
) -> Result<StreamChunk, StreamShapeError> {
    let arguments = chunk_arguments(chunk)?;
    let instant = shaper.identity.instant();
    let response_id = adopted_id(shaper, arguments.id);
    let created = adopted_created(shaper, arguments.created, instant.seconds);
    let system_fingerprint = shaper
        .system_fingerprint
        .clone()
        .or(arguments.system_fingerprint);
    let merged = merged_hidden_params(shaper, hidden_params, instant.timestamp);
    let mut choices = arguments.choices.unwrap_or_default();
    if choices.is_empty() {
        choices.push(StreamChoice::default());
    }
    Ok(StreamChunk {
        id: response_id,
        object: "chat.completion.chunk".to_owned(),
        created,
        model: shaper.model.clone(),
        system_fingerprint,
        choices,
        usage: arguments.usage,
        provider_specific_fields: arguments.provider_specific_fields,
        hidden_params: merged,
        extra: arguments.extra,
    })
}

pub fn set_model_id(shaper: &mut ChunkShaper, id: &str, model_response: &mut StreamChunk) {
    let usable = !id.trim().is_empty();
    if shaper.response_id.is_none() && usable {
        shaper.response_id = Some(id.to_owned());
    }
    if usable {
        model_response.hidden_params.insert(
            "received_model_id".to_owned(),
            JsonValue::String(id.to_owned()),
        );
    }
    if let Some(response_id) = &shaper.response_id {
        model_response.id.clone_from(response_id);
    }
}

fn chunk_arguments(chunk: Option<&JsonObject>) -> Result<ChunkArguments, StreamShapeError> {
    let Some(object) = chunk else {
        return Ok(ChunkArguments::default());
    };
    let mut filtered = object.clone();
    filtered.remove("model");
    filtered.remove("stream");
    let value = to_value(JsonValue::Object(filtered))?;
    Ok(from_value(value)?)
}

fn adopted_id(shaper: &mut ChunkShaper, chunk_id: Option<String>) -> String {
    let response_id = match (&shaper.response_id, chunk_id) {
        (Some(response_id), _) => response_id.clone(),
        (None, Some(provided)) => provided,
        (None, None) => format!("chatcmpl-{}", shaper.identity.new_id()),
    };
    if shaper.response_id.is_none() && !response_id.is_empty() {
        shaper.response_id = Some(response_id.clone());
    }
    response_id
}

fn adopted_created(shaper: &mut ChunkShaper, chunk_created: Option<i64>, now: i64) -> i64 {
    if let Some(created) = shaper.created {
        return created;
    }
    let created = chunk_created.unwrap_or(now);
    shaper.created = Some(created);
    created
}

fn merged_hidden_params(
    shaper: &ChunkShaper,
    hidden_params: Option<&JsonObject>,
    timestamp: f64,
) -> JsonObject {
    let mut merged = hidden_params.cloned().unwrap_or_default();
    let provider = shaper
        .provider
        .as_ref()
        .map_or(JsonValue::Null, |provider| {
            JsonValue::String(provider.clone())
        });
    merged.insert("custom_llm_provider".to_owned(), provider);
    let created_at = Number::from_f64(timestamp).map_or(JsonValue::Null, JsonValue::Number);
    merged.insert("created_at".to_owned(), created_at);
    for (key, value) in &shaper.base_hidden_params {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

pub fn copy_provider_specific_fields(
    original_chunk: &StreamChunk,
    model_response: &mut StreamChunk,
) {
    let Some(fields) = &original_chunk.provider_specific_fields else {
        return;
    };
    model_response.provider_specific_fields = Some(fields.clone());
    for (key, value) in fields {
        model_response.extra.insert(key.clone(), value.clone());
    }
}

pub fn stamp_served_service_tier(
    response: &mut StreamChunk,
    complete_streaming_response: &StreamChunk,
) {
    let served_tier = complete_streaming_response
        .extra
        .get("service_tier")
        .and_then(JsonValue::as_str)
        .filter(|tier| !tier.is_empty());
    if let Some(tier) = served_tier {
        response.extra.insert(
            "service_tier".to_owned(),
            JsonValue::String(tier.to_owned()),
        );
    }
}

#[must_use]
pub const fn provider_response_model(chunk: &StreamChunk) -> Option<&str> {
    if chunk.model.is_empty() {
        return None;
    }
    Some(chunk.model.as_str())
}

#[must_use]
pub fn parsed_provider_hidden_params(hidden: &JsonObject) -> Option<ParsedChunkHiddenParams> {
    match hidden.get("provider_specific_fields") {
        None | Some(JsonValue::Null) => Some(ParsedChunkHiddenParams {
            provider_specific_fields: None,
        }),
        Some(JsonValue::Object(fields)) => Some(ParsedChunkHiddenParams {
            provider_specific_fields: Some(fields.clone()),
        }),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => None,
    }
}

#[must_use]
pub fn provider_hidden_params(
    chunk: &StreamChunk,
    provider_response_model: Option<&str>,
) -> Option<JsonObject> {
    let provider_fields = parsed_provider_hidden_params(&chunk.hidden_params)
        .and_then(|parsed| parsed.provider_specific_fields)
        .filter(|fields| !fields.is_empty());
    let mut params = JsonObject::new();
    if let Some(model) = provider_response_model {
        params.insert(
            "provider_response_model".to_owned(),
            JsonValue::String(model.to_owned()),
        );
    }
    if let Some(fields) = provider_fields {
        params.insert(
            "provider_specific_fields".to_owned(),
            JsonValue::Object(fields),
        );
    }
    if params.is_empty() {
        return None;
    }
    Some(params)
}
