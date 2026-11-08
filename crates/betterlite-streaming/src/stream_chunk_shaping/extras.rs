use uuid::Uuid;

use crate::stream_chunk_shaping::creation::StreamInstant;
use crate::stream_chunk_shaping::records::{
    GENERIC_CHUNK_FIELDS, GenericChunk, JsonObject, JsonValue, StreamChoice, StreamChunk,
    StreamDelta,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SpecialDeltaAttribute {
    Audio,
    Images,
}

const SPECIAL_ATTRIBUTES: [SpecialDeltaAttribute; 2] =
    [SpecialDeltaAttribute::Audio, SpecialDeltaAttribute::Images];

#[must_use]
pub fn has_special_delta_content(model_response: &StreamChunk) -> bool {
    let Some(choice) = model_response.choices.first() else {
        return false;
    };
    let delta = &choice.delta;
    delta.tool_calls.is_some()
        || delta.function_call.is_some()
        || delta.audio.is_some()
        || delta.images.is_some()
}

#[must_use]
pub const fn has_special_delta_attribute(
    delta: &StreamDelta,
    attribute: SpecialDeltaAttribute,
) -> bool {
    match attribute {
        SpecialDeltaAttribute::Audio => delta.audio.is_some(),
        SpecialDeltaAttribute::Images => delta.images.is_some(),
    }
}

pub fn copy_delta_attribute(
    source: &StreamDelta,
    target: &mut StreamDelta,
    attribute: SpecialDeltaAttribute,
) {
    match attribute {
        SpecialDeltaAttribute::Audio => target.audio.clone_from(&source.audio),
        SpecialDeltaAttribute::Images => target.images.clone_from(&source.images),
    }
}

#[must_use]
pub fn has_any_special_attributes(delta: &StreamDelta) -> bool {
    SPECIAL_ATTRIBUTES
        .into_iter()
        .any(|attribute| has_special_delta_attribute(delta, attribute))
}

pub fn apply_special_delta_attributes(delta: &StreamDelta, model_response: &mut StreamChunk) {
    let Some(choice) = model_response.choices.first_mut() else {
        return;
    };
    for attribute in SPECIAL_ATTRIBUTES {
        if has_special_delta_attribute(delta, attribute) {
            copy_delta_attribute(delta, &mut choice.delta, attribute);
        }
    }
}

#[must_use]
pub fn generic_chunk_has_known_fields(chunk: &JsonObject) -> bool {
    chunk
        .keys()
        .all(|key| GENERIC_CHUNK_FIELDS.split(' ').any(|field| field == key))
}

#[must_use]
pub fn convert_generic_chunk(
    chunk: &GenericChunk,
    now: fn() -> StreamInstant,
    next_id: fn() -> Uuid,
) -> StreamChunk {
    let instant = now();
    let delta = StreamDelta {
        content: Some(chunk.text.clone()),
        role: None,
        function_call: None,
        tool_calls: chunk.tool_use.clone(),
        audio: None,
        images: None,
        annotations: None,
        reasoning_content: None,
        thinking_blocks: None,
        provider_specific_fields: None,
        extra: JsonObject::new(),
    };
    let choice = StreamChoice {
        index: chunk.index.unwrap_or(0),
        delta,
        finish_reason: None,
        logprobs: None,
        extra: JsonObject::new(),
    };
    let finish_reason = if chunk.is_finished {
        JsonValue::String(chunk.finish_reason.clone())
    } else {
        JsonValue::Null
    };
    let mut extra = JsonObject::new();
    extra.insert("finish_reason".to_owned(), finish_reason);
    StreamChunk {
        id: next_id().to_string(),
        object: "chat.completion.chunk".to_owned(),
        created: instant.seconds,
        model: String::new(),
        system_fingerprint: None,
        choices: vec![choice],
        usage: chunk.usage.clone(),
        provider_specific_fields: None,
        hidden_params: JsonObject::new(),
        extra,
    }
}

#[must_use]
pub fn strip_sse_data_from_chunk(chunk: Option<&str>) -> Option<&str> {
    let line = chunk?;
    line.strip_prefix("data: ")
        .or_else(|| line.strip_prefix("data:"))
        .or(Some(line))
}
