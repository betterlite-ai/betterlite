use crate::stream_chunk_shaping::creation::StreamIdentity;
use crate::stream_chunk_shaping::records::{
    GENERIC_CHUNK_FIELDS, GenericChunk, JsonObject, JsonValue, StreamChoice, StreamChunk,
    StreamDelta,
};

#[must_use]
pub fn generic_chunk_has_known_fields(chunk: &JsonObject) -> bool {
    for key in chunk.keys() {
        if !GENERIC_CHUNK_FIELDS.contains(&key.as_str()) {
            return false;
        }
    }
    true
}

#[must_use]
pub fn convert_generic_chunk(chunk: &GenericChunk, identity: &dyn StreamIdentity) -> StreamChunk {
    let instant = identity.instant();
    let delta = StreamDelta {
        content: Some(chunk.text.clone()),
        tool_calls: chunk.tool_use.clone(),
        role: None,
        function_call: None,
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
        id: identity.new_id(),
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
