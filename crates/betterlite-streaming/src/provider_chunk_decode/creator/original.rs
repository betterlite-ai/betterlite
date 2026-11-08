use crate::provider_chunk_decode::chunk::{DecodeState, DecodedChunk, ProviderChunk, as_object};
use crate::provider_chunk_decode::creator::fields::apply_provider_fields;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::provider_hidden_params;
use crate::stream_chunk_shaping::extras::{
    apply_special_delta_attributes, has_any_special_attributes,
};
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk, StreamDelta};

pub(super) fn normalize_original(original: &mut StreamChunk) {
    let Some(choice) = original.choices.first_mut() else {
        return;
    };
    let delta = &mut choice.delta;
    if let Some(JsonValue::Object(function)) = &mut delta.function_call
        && matches!(function.get("arguments"), None | Some(JsonValue::Null))
    {
        function.insert("arguments".to_owned(), JsonValue::String(String::new()));
    }
    if let Some(JsonValue::Array(tools)) = &mut delta.tool_calls {
        for tool in tools {
            normalize_tool(tool);
        }
    }
}

fn normalize_tool(tool: &mut JsonValue) {
    let JsonValue::Object(fields) = tool else {
        return;
    };
    let needs_type = matches!(fields.get("type"), None | Some(JsonValue::Null));
    let has_function = fields
        .get("function")
        .is_some_and(|function| as_object(function).is_some());
    if needs_type && has_function {
        fields.insert("type".to_owned(), JsonValue::String("function".to_owned()));
    }
}

pub(super) fn sync_finish(shaper: &mut ChunkShaper, state: &DecodeState) {
    shaper
        .received_finish_reason
        .clone_from(&state.received_finish_reason);
    shaper
        .intermittent_finish_reason
        .clone_from(&state.intermittent_finish_reason);
}

pub(super) fn fill_decoded(
    chunk: &ProviderChunk,
    state: &DecodeState,
    shaper: &mut ChunkShaper,
    decoded: &DecodedChunk,
    model_response: &mut StreamChunk,
) {
    shaper.model.clone_from(&state.model);
    model_response.model.clone_from(&state.model);
    if let Some(fingerprint) = &decoded.system_fingerprint {
        shaper.system_fingerprint.clone_from(fingerprint);
    }
    if let Some(fingerprint) = &decoded.response_system_fingerprint {
        model_response.system_fingerprint.clone_from(fingerprint);
    }
    model_response.usage.clone_from(&decoded.usage);
    if model_response.usage.is_none() && chunk.is_openai_chunk() {
        model_response.usage = chunk.openai_chunk().ok().and_then(|stream| stream.usage);
    }
    if let Some(logprobs) = &decoded.logprobs
        && let Some(choice) = model_response.choices.first_mut()
    {
        choice.logprobs = Some(logprobs.clone());
    }
    if let Some(fields) = &decoded.provider_specific_fields {
        apply_provider_fields(model_response, fields);
    }
}

pub(super) fn fill_completion(completion_obj: &mut StreamDelta, decoded: &DecodedChunk) {
    completion_obj.content = Some(decoded.text.clone());
    completion_obj.tool_calls.clone_from(&decoded.tool_calls);
}

pub(super) fn hidden_params_of(chunk: &ProviderChunk) -> Option<JsonObject> {
    let provider_model = chunk.model_name();
    if chunk.is_openai_chunk() {
        let stream = chunk.openai_chunk().ok()?;
        return provider_hidden_params(&stream, provider_model);
    }
    provider_model.map(|model| {
        let mut params = JsonObject::new();
        params.insert(
            "provider_response_model".to_owned(),
            JsonValue::String(model.to_owned()),
        );
        params
    })
}

pub(super) fn fill_tool_use(
    shaper: &mut ChunkShaper,
    state: &DecodeState,
    completion_obj: &mut StreamDelta,
) -> Result<(), ProviderChunkError> {
    if state.options.is_function_call {
        let function = first_function(completion_obj.tool_calls.as_ref())?;
        completion_obj.function_call = Some(function);
        completion_obj.tool_calls = None;
    }
    shaper.tool_call = true;
    Ok(())
}

pub(super) fn apply_original_choice(model_response: &mut StreamChunk, original: &StreamChunk) {
    let Some(choice) = original.choices.first() else {
        return;
    };
    let delta = &choice.delta;
    if matches!(
        (&delta.function_call, &delta.tool_calls),
        (Some(_), _) | (_, Some(_))
    ) {
        fill_function_call(model_response, original, delta);
    } else if has_any_special_attributes(delta) {
        apply_special_delta_attributes(delta, model_response);
    } else {
        copy_plain_delta(model_response, delta);
    }
}

fn copy_plain_delta(model_response: &mut StreamChunk, delta: &StreamDelta) {
    if let Some(target) = model_response.choices.first_mut() {
        target.delta = delta.clone();
    }
}

fn fill_function_call(
    model_response: &mut StreamChunk,
    original: &StreamChunk,
    delta: &StreamDelta,
) {
    model_response
        .system_fingerprint
        .clone_from(&original.system_fingerprint);
    let mut normalized = delta.clone();
    if normalized.role.is_none() {
        normalized.role = Some("assistant".to_owned());
    }
    if let Some(target) = model_response.choices.first_mut() {
        target.delta = normalized;
    }
}

fn first_function(calls: Option<&JsonValue>) -> Result<JsonValue, ProviderChunkError> {
    let Some(JsonValue::Array(items)) = calls else {
        return Err(ProviderChunkError::MissingField {
            field: "tool_calls",
        });
    };
    let Some(JsonValue::Object(tool)) = items.first() else {
        return Err(ProviderChunkError::MissingField {
            field: "tool_calls[0]",
        });
    };
    tool.get("function")
        .cloned()
        .ok_or(ProviderChunkError::MissingField { field: "function" })
}
