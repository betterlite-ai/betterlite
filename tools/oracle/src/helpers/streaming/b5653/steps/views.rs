use betterlite_streaming::provider_chunk_decode::chunk::{DecodeState, DecodedChunk};
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, Usage,
};
use serde_json::{Value, json};

pub(super) fn decode_view(decoded: &DecodedChunk, state: &DecodeState) -> Value {
    json!({
        "early_return": decoded.early_return,
        "early_chunk": early_view(decoded),
        "text": decoded.text,
        "is_finished": decoded.is_finished,
        "finish_reason": reason_view(decoded.finish_reason.as_deref()),
        "usage": decoded.usage.as_ref().map_or(Value::Null, usage_view),
        "tool_use": json_value(decoded.tool_use.as_ref()),
        "provider_specific_fields": object_value(decoded.provider_specific_fields.as_ref()),
        "tool_calls": json_value(decoded.tool_calls.as_ref()),
        "state": state_view(state),
    })
}

fn early_view(decoded: &DecodedChunk) -> Value {
    if !decoded.early_return {
        return Value::Null;
    }
    decoded.original.as_ref().map_or(Value::Null, chunk_view)
}

fn chunk_view(chunk: &StreamChunk) -> Value {
    let choices: Vec<Value> = chunk
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": choice.delta.content,
            })
        })
        .collect();
    json!({"id": chunk.id, "choices": choices})
}

fn state_view(state: &DecodeState) -> Value {
    json!({
        "model": state.model,
        "received_finish_reason": state.received_finish_reason,
        "intermittent_finish_reason": state.intermittent_finish_reason,
        "completion_stream": state.completion_stream,
    })
}

fn reason_view(reason: Option<&str>) -> Value {
    match reason {
        Some(text) if !text.is_empty() => Value::String(text.to_owned()),
        Some(_) | None => Value::Null,
    }
}

fn usage_view(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    })
}

fn json_value(value: Option<&JsonValue>) -> Value {
    value.map_or(Value::Null, |found| {
        serde_json::to_value(found).unwrap_or(Value::Null)
    })
}

fn object_value(value: Option<&JsonObject>) -> Value {
    value.map_or(Value::Null, |found| {
        serde_json::to_value(found).unwrap_or(Value::Null)
    })
}

pub(super) fn creator_view(
    created: Option<&StreamChunk>,
    shaper: &ChunkShaper,
    state: &DecodeState,
) -> Value {
    json!({
        "chunk": created.map_or(Value::Null, summary_view),
        "received_finish_reason": state.received_finish_reason,
        "intermittent_finish_reason": state.intermittent_finish_reason,
        "sent_first_chunk": shaper.progress.sent_first_chunk,
        "tool_call": shaper.tool_call,
        "chunks": shaper.chunks.len(),
    })
}

fn summary_view(chunk: &StreamChunk) -> Value {
    let choice = chunk.choices.first();
    json!({
        "id": chunk.id,
        "model": chunk.model,
        "created": chunk.created,
        "system_fingerprint": chunk.system_fingerprint,
        "finish_reason": choice.and_then(|first| first.finish_reason.clone()),
        "content": choice.and_then(|first| first.delta.content.clone()),
        "role": choice.and_then(|first| first.delta.role.clone()),
        "function_call": choice.map_or(Value::Null, |first| {
            first.delta.function_call.as_ref().map_or(Value::Null, function_view)
        }),
        "tool_calls": choice.map_or(Value::Null, |first| {
            first.delta.tool_calls.as_ref().map_or(Value::Null, calls_view)
        }),
        "choices": chunk.choices.len(),
        "usage": chunk.usage.as_ref().map_or(Value::Null, usage_view),
        "provider_specific_fields": object_value(chunk.provider_specific_fields.as_ref()),
    })
}

fn function_view(value: &JsonValue) -> Value {
    match value {
        JsonValue::Object(fields) => json!({
            "name": fields.get("name"),
            "arguments": fields.get("arguments"),
        }),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Value::Null,
    }
}

fn calls_view(value: &JsonValue) -> Value {
    match value {
        JsonValue::Array(items) => Value::Array(items.iter().map(tool_call_view).collect()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Object(_) => Value::Null,
    }
}

fn tool_call_view(value: &JsonValue) -> Value {
    let fields = match value {
        JsonValue::Object(fields) => Some(fields),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    };
    json!({
        "id": fields.and_then(|found| found.get("id")),
        "type": fields.and_then(|found| found.get("type")),
        "function": fields
            .and_then(|found| found.get("function"))
            .map_or(Value::Null, function_view),
    })
}

pub(super) fn error_view(error: &ProviderChunkError) -> Value {
    match error {
        ProviderChunkError::StopIteration => json!({"raised": {"message": ""}}),
        ProviderChunkError::ParseResponse { .. }
        | ProviderChunkError::RejectedResponse { .. }
        | ProviderChunkError::ProviderReported { .. }
        | ProviderChunkError::UnknownStream
        | ProviderChunkError::VertexBlocked
        | ProviderChunkError::NotAString { .. }
        | ProviderChunkError::UnexpectedChunk { .. }
        | ProviderChunkError::MissingField { .. }
        | ProviderChunkError::InvalidField { .. }
        | ProviderChunkError::InvalidUtf8 { .. }
        | ProviderChunkError::StreamingFailure { .. }
        | ProviderChunkError::InvalidChunk { .. }
        | ProviderChunkError::Shape { .. } => json!({"raised": {"message": error.to_string()}}),
    }
}
