use std::mem;

use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::StreamShapeError;
use crate::stream_chunk_shaping::extras::has_special_delta_content;
use crate::stream_chunk_shaping::records::{
    ChunkResponseObject, JsonValue, StreamChunk, StreamDelta, item_count,
};
use crate::stream_chunk_shaping::tokens::{
    check_special_tokens, mapped_finish_reason, raise_on_model_repetition,
};

#[must_use]
pub fn is_delta_empty(delta: &StreamDelta) -> bool {
    delta.content.as_deref().is_none_or(str::is_empty)
        && delta.tool_calls.is_none()
        && delta.function_call.is_none()
}

#[must_use]
pub fn is_chunk_non_empty(
    shaper: &ChunkShaper,
    completion_obj: &StreamDelta,
    model_response: &StreamChunk,
    response_obj: &ChunkResponseObject,
) -> bool {
    let Some(choice) = model_response.choices.first() else {
        return false;
    };
    let delta = &choice.delta;
    let checks = [
        completion_obj
            .content
            .as_deref()
            .is_some_and(|text| !text.is_empty()),
        completion_obj
            .tool_calls
            .as_ref()
            .is_some_and(|calls| item_count(calls) > 0),
        completion_obj.function_call.is_some(),
        delta
            .tool_calls
            .as_ref()
            .is_some_and(|calls| item_count(calls) > 0),
        delta.function_call.is_some(),
        delta.reasoning_content.is_some(),
        delta.provider_specific_fields.is_some(),
        response_obj.provider_specific_fields.is_some(),
        delta.annotations.is_some(),
        !shaper.progress.sent_first_chunk && delta.role.is_some(),
        delta
            .extra
            .get("reasoning_items")
            .is_some_and(|items| !matches!(items, JsonValue::Null)),
    ];
    checks.into_iter().any(|flag| flag)
}

pub fn strip_role_from_delta(shaper: &mut ChunkShaper, model_response: &mut StreamChunk) {
    let Some(choice) = model_response.choices.first_mut() else {
        return;
    };
    if shaper.progress.sent_first_chunk {
        choice.delta.role = None;
        return;
    }
    choice.delta.role = Some("assistant".to_owned());
    shaper.progress.sent_first_chunk = true;
}

#[must_use]
pub fn handle_special_delta_content(
    shaper: &mut ChunkShaper,
    mut model_response: StreamChunk,
) -> StreamChunk {
    strip_role_from_delta(shaper, &mut model_response);
    model_response
}

pub fn combine_thinking_in_choices(shaper: &mut ChunkShaper, model_response: &mut StreamChunk) {
    if !shaper.merge_reasoning_in_choices {
        return;
    }
    let Some(choice) = model_response.choices.first_mut() else {
        return;
    };
    let reasoning = choice
        .delta
        .reasoning_content
        .take()
        .filter(|text| !text.is_empty());
    if let Some(thought) = reasoning {
        let opened = shaper.thinking.sent_first_block;
        let existing = choice.delta.content.take().unwrap_or_default();
        let content = if opened {
            thought
        } else {
            format!("{existing}<think>{thought}")
        };
        shaper.thinking.sent_first_block = true;
        choice.delta.content = Some(content);
        return;
    }
    if !shaper.thinking.sent_first_block || shaper.thinking.sent_last_block {
        return;
    }
    let Some(visible) = choice.delta.content.take().filter(|text| !text.is_empty()) else {
        return;
    };
    choice.delta.content = Some(format!("</think>{visible}"));
    shaper.thinking.sent_last_block = true;
}

pub fn return_processed_chunk_logic(
    shaper: &mut ChunkShaper,
    completion_obj: &mut StreamDelta,
    mut model_response: StreamChunk,
    response_obj: &ChunkResponseObject,
    repeated_chunk_limit: usize,
) -> Result<Option<StreamChunk>, StreamShapeError> {
    if is_chunk_non_empty(shaper, completion_obj, &model_response, response_obj) {
        raise_on_model_repetition(shaper, repeated_chunk_limit)?;
        let reason = model_response
            .choices
            .first()
            .and_then(|choice| choice.finish_reason.clone());
        let text = completion_obj.content.as_deref().unwrap_or_default();
        let (hold, processed) = check_special_tokens(shaper, text, reason.as_deref());
        if hold {
            return Ok(None);
        }
        if !fill_processed_choice(
            shaper,
            completion_obj,
            &mut model_response,
            response_obj,
            processed,
        ) {
            return Ok(None);
        }
        combine_thinking_in_choices(shaper, &mut model_response);
        return Ok(Some(model_response));
    }
    if shaper.received_finish_reason.is_some() {
        return Ok(finish_chunk(shaper, model_response, response_obj));
    }
    if has_special_delta_content(&model_response) {
        return Ok(Some(handle_special_delta_content(shaper, model_response)));
    }
    if model_response.usage.is_some() {
        shaper.chunks.push(model_response);
    }
    Ok(None)
}

fn fill_processed_choice(
    shaper: &mut ChunkShaper,
    completion_obj: &mut StreamDelta,
    model_response: &mut StreamChunk,
    response_obj: &ChunkResponseObject,
    processed: String,
) -> bool {
    if let Some(original_chunk) = &response_obj.original_chunk {
        return take_original_choice(shaper, model_response, original_chunk);
    }
    completion_obj.content = Some(processed);
    if !shaper.progress.sent_first_chunk {
        completion_obj.role = Some("assistant".to_owned());
        shaper.progress.sent_first_chunk = true;
    }
    if let Some(fields) = &response_obj.provider_specific_fields {
        completion_obj.provider_specific_fields = Some(fields.clone());
    }
    let requested_index = completion_obj
        .extra
        .get("index")
        .and_then(JsonValue::as_i64);
    let Some(choice) = model_response.choices.first_mut() else {
        return false;
    };
    choice.delta.clone_from(completion_obj);
    if let Some(index) = requested_index {
        choice.index = index;
    }
    true
}

fn take_original_choice(
    shaper: &mut ChunkShaper,
    model_response: &mut StreamChunk,
    original_chunk: &StreamChunk,
) -> bool {
    if original_chunk.choices.is_empty() {
        return false;
    }
    let mut choices = original_chunk.choices.clone();
    for choice in &mut choices {
        choice.finish_reason = None;
    }
    model_response.choices = choices;
    model_response
        .system_fingerprint
        .clone_from(&original_chunk.system_fingerprint);
    let citations = original_chunk
        .extra
        .get("citations")
        .cloned()
        .unwrap_or(JsonValue::Null);
    model_response
        .extra
        .insert("citations".to_owned(), citations);
    let mut preserved = original_chunk.extra.clone();
    preserved.remove("citations");
    model_response.extra.extend(preserved);
    strip_role_from_delta(shaper, model_response);
    true
}

fn finish_chunk(
    shaper: &mut ChunkShaper,
    mut model_response: StreamChunk,
    response_obj: &ChunkResponseObject,
) -> Option<StreamChunk> {
    let reason = shaper.received_finish_reason.clone().unwrap_or_default();
    if shaper.progress.sent_last_chunk {
        // Bedrock sends the guardrail trace in a chunk after the finish reason, and the caller must see it.
        if shaper.provider.as_deref() == Some("bedrock")
            && model_response.extra.contains_key("trace")
        {
            return Some(model_response);
        }
        return model_response.usage.is_some().then_some(model_response);
    }
    if !shaper.holding_chunk.is_empty() {
        let held = mem::take(&mut shaper.holding_chunk);
        if let Some(choice) = model_response.choices.first_mut() {
            let mut flushed = held;
            if let Some(content) = choice.delta.content.take() {
                flushed.push_str(&content);
            }
            choice.delta.content = Some(flushed);
        }
    }
    if let Some(original_chunk) = &response_obj.original_chunk {
        model_response.extra.extend(original_chunk.extra.clone());
    }
    let Some(choice) = model_response.choices.first_mut() else {
        return Some(model_response);
    };
    if !is_delta_empty(&choice.delta) {
        return Some(model_response);
    }
    choice.delta = StreamDelta::default();
    choice.finish_reason = Some(mapped_finish_reason(&reason).to_owned());
    shaper.progress.sent_last_chunk = true;
    Some(model_response)
}
