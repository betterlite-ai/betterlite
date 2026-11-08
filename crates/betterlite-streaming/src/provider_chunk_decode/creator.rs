use crate::provider_chunk_decode::chunk::{DecodeState, DecodedChunk, ProviderChunk, as_object};
use crate::provider_chunk_decode::dispatch::decode_provider_chunk;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::{
    copy_provider_specific_fields, model_response_creator, provider_hidden_params, set_model_id,
};
use crate::stream_chunk_shaping::decision::return_processed_chunk_logic;
use crate::stream_chunk_shaping::extras::{
    apply_special_delta_attributes, has_any_special_attributes,
};
use crate::stream_chunk_shaping::records::{
    ChunkResponseObject, JsonObject, JsonValue, StreamChoice, StreamChunk, StreamDelta, Usage,
    item_count,
};
use crate::stream_chunk_shaping::usage_totals::record_usage_only_chunk;

pub fn chunk_creator(
    shaper: &mut ChunkShaper,
    state: &mut DecodeState,
    chunk: &ProviderChunk,
) -> Result<Option<StreamChunk>, ProviderChunkError> {
    if let Some(id) = chunk.response_id() {
        shaper.response_id = Some(id.to_owned());
    }
    let hidden = hidden_params_of(chunk);
    let mut model_response = model_response_creator(shaper, None, hidden.as_ref())?;
    let decoded = decode_provider_chunk(shaper.provider.as_deref(), chunk, state)?;
    if decoded.early_return {
        return Ok(decoded.original);
    }
    fill_decoded(state, shaper, &decoded, &mut model_response);
    let response_obj = ChunkResponseObject {
        original_chunk: decoded.original.clone(),
        provider_specific_fields: decoded.provider_specific_fields.clone(),
    };
    let mut completion_obj = StreamDelta::default();
    fill_completion(&mut completion_obj, &decoded);
    let processed = match prepare_original(shaper, state, &response_obj, model_response) {
        Prepared::Continue(response) => response,
        Prepared::Emit(ready) => return Ok(Some(ready)),
        Prepared::Skip => return Ok(None),
    };
    if completion_obj
        .tool_calls
        .as_ref()
        .is_some_and(|calls| item_count(calls) > 0)
    {
        fill_tool_use(shaper, state, &mut completion_obj)?;
    }
    sync_finish(shaper, state);
    let result = return_processed_chunk_logic(
        shaper,
        &mut completion_obj,
        processed,
        &response_obj,
        state.repeated_chunk_limit,
    )?;
    state.sent_first_chunk = shaper.progress.sent_first_chunk;
    Ok(result)
}

enum Prepared {
    Continue(StreamChunk),
    Emit(StreamChunk),
    Skip,
}

fn prepare_original(
    shaper: &mut ChunkShaper,
    state: &mut DecodeState,
    response_obj: &ChunkResponseObject,
    mut model_response: StreamChunk,
) -> Prepared {
    let Some(original) = &response_obj.original_chunk else {
        return Prepared::Continue(model_response);
    };
    set_model_id(shaper, &original.id, &mut model_response);
    copy_provider_specific_fields(original, &mut model_response);
    if !original.choices.is_empty() {
        apply_original_choice(&mut model_response, original);
        return Prepared::Continue(model_response);
    }
    state.sent_first_chunk = shaper.progress.sent_first_chunk;
    if state.options.send_stream_usage {
        model_response.choices.clear();
        return Prepared::Emit(model_response);
    }
    record_usage_only_chunk(shaper, model_response);
    Prepared::Skip
}

fn sync_finish(shaper: &mut ChunkShaper, state: &DecodeState) {
    shaper
        .received_finish_reason
        .clone_from(&state.received_finish_reason);
    shaper
        .intermittent_finish_reason
        .clone_from(&state.intermittent_finish_reason);
}

fn fill_decoded(
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
    if let Some(logprobs) = &decoded.logprobs
        && let Some(choice) = model_response.choices.first_mut()
    {
        choice.logprobs = Some(logprobs.clone());
    }
    if let Some(fields) = &decoded.provider_specific_fields {
        apply_provider_fields(model_response, fields);
    }
}

fn fill_completion(completion_obj: &mut StreamDelta, decoded: &DecodedChunk) {
    completion_obj.content = Some(decoded.text.clone());
    completion_obj.tool_calls.clone_from(&decoded.tool_calls);
}

fn hidden_params_of(chunk: &ProviderChunk) -> Option<JsonObject> {
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

fn fill_tool_use(
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

fn apply_original_choice(model_response: &mut StreamChunk, original: &StreamChunk) {
    let Some(choice) = original.choices.first() else {
        return;
    };
    let delta = &choice.delta;
    if delta.function_call.is_some() || delta.tool_calls.is_some() {
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
    if let Some(JsonValue::Object(function)) = &mut normalized.function_call
        && matches!(function.get("arguments"), None | Some(JsonValue::Null))
    {
        function.insert("arguments".to_owned(), JsonValue::String(String::new()));
    }
    if normalized.role.is_none() {
        normalized.role = Some("assistant".to_owned());
    }
    if let Some(JsonValue::Array(tools)) = &mut normalized.tool_calls {
        for tool in tools {
            normalize_tool_type(tool);
        }
    }
    if let Some(target) = model_response.choices.first_mut() {
        target.delta = normalized;
    }
}

fn normalize_tool_type(tool: &mut JsonValue) {
    let JsonValue::Object(fields) = tool else {
        return;
    };
    if fields
        .get("function")
        .is_none_or(|function| as_object(function).is_none())
    {
        return;
    }
    if matches!(fields.get("type"), None | Some(JsonValue::Null)) {
        fields.insert("type".to_owned(), JsonValue::String("function".to_owned()));
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

fn apply_provider_fields(model_response: &mut StreamChunk, fields: &JsonObject) {
    for (key, value) in fields {
        if !apply_declared_field(model_response, key, value) {
            model_response.extra.insert(key.clone(), value.clone());
        }
    }
}

fn apply_declared_field(model_response: &mut StreamChunk, key: &str, value: &JsonValue) -> bool {
    match (key, value) {
        ("id", JsonValue::String(text)) => model_response.id.clone_from(text),
        ("object", JsonValue::String(text)) => model_response.object.clone_from(text),
        ("model", JsonValue::String(text)) => model_response.model.clone_from(text),
        ("created", JsonValue::Number(number)) => {
            model_response.created = number.as_i64().unwrap_or(model_response.created);
        }
        ("system_fingerprint", fingerprint) => {
            model_response.system_fingerprint = string_of(fingerprint);
        }
        ("usage", counted) => model_response.usage = usage_of_value(counted),
        ("provider_specific_fields", nested) => {
            model_response.provider_specific_fields = object_of_value(nested);
        }
        ("hidden_params", JsonValue::Object(hidden)) => {
            model_response.hidden_params.clone_from(hidden);
        }
        ("choices", JsonValue::Array(choices)) => model_response.choices = choices_of(choices),
        _ => return false,
    }
    true
}

fn string_of(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

fn object_of_value(value: &JsonValue) -> Option<JsonObject> {
    match value {
        JsonValue::Object(fields) => Some(fields.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn usage_of_value(value: &JsonValue) -> Option<Usage> {
    match value {
        JsonValue::Object(fields) => Some(usage_of(fields)),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn usage_of(fields: &JsonObject) -> Usage {
    Usage {
        prompt_tokens: count_of(fields, "prompt_tokens"),
        completion_tokens: count_of(fields, "completion_tokens"),
        total_tokens: count_of(fields, "total_tokens"),
        cost: None,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }
}

fn count_of(fields: &JsonObject, key: &str) -> i64 {
    fields
        .get(key)
        .and_then(JsonValue::as_i64)
        .unwrap_or_default()
}

fn choices_of(items: &[JsonValue]) -> Vec<StreamChoice> {
    items
        .iter()
        .filter_map(|item| {
            let plain = serde_json::to_value(item).ok()?;
            serde_json::from_value(plain).ok()
        })
        .collect()
}
