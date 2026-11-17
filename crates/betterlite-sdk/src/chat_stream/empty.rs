use betterlite_streaming::stream_chunk_shaping::records::{
    JsonValue, StreamChoice, StreamChunk, StreamDelta,
};

// is_model_response_stream_empty: a stripped usage chunk still counts when anything else carries data.
pub(super) fn is_empty(chunk: &StreamChunk) -> bool {
    if chunk.usage.is_some() {
        return false;
    }
    if chunk
        .provider_specific_fields
        .as_ref()
        .is_some_and(|fields| !fields.is_empty())
    {
        return false;
    }
    if chunk.extra.values().any(value_is_meaningful) {
        return false;
    }
    !chunk.choices.iter().any(choice_is_meaningful)
}

fn choice_is_meaningful(choice: &StreamChoice) -> bool {
    if choice.finish_reason.is_some() || choice.logprobs.is_some() || choice.index != 0 {
        return true;
    }
    if delta_is_meaningful(&choice.delta) {
        return true;
    }
    choice.extra.values().any(value_is_meaningful)
}

fn delta_is_meaningful(delta: &StreamDelta) -> bool {
    let checks = [
        delta
            .content
            .as_deref()
            .is_some_and(|text| !text.is_empty()),
        delta.role.as_deref().is_some_and(|role| !role.is_empty()),
        delta
            .function_call
            .as_ref()
            .is_some_and(value_is_meaningful),
        delta.tool_calls.as_ref().is_some_and(value_is_meaningful),
        delta.audio.as_ref().is_some_and(value_is_meaningful),
        delta.images.as_ref().is_some_and(value_is_meaningful),
        delta.annotations.as_ref().is_some_and(value_is_meaningful),
        delta
            .reasoning_content
            .as_deref()
            .is_some_and(|text| !text.is_empty()),
        delta
            .thinking_blocks
            .as_ref()
            .is_some_and(value_is_meaningful),
        delta
            .provider_specific_fields
            .as_ref()
            .is_some_and(|fields| !fields.is_empty()),
    ];
    checks.into_iter().any(|flag| flag) || delta.extra.values().any(value_is_meaningful)
}

// _has_meaningful_content: None and the empty JSON values carry nothing, any other value does.
fn value_is_meaningful(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(_) | JsonValue::Number(_) => true,
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}
