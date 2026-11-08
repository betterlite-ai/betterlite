use betterlite_streaming::provider_chunk_decode::chunk::DecodeState;
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk};
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::views::{error_view, object_value, usage_view};
use super::{Harness, chunk_of};

pub(super) fn creator_step(harness: &mut Harness, step: &Value) -> Result<Value, Box<HelperError>> {
    let chunk = chunk_of(step)?;
    let created = chunk_creator(&mut harness.shaper, &mut harness.state, &chunk);
    match created {
        Ok(created_chunk) => Ok(creator_view(
            created_chunk.as_ref(),
            &harness.shaper,
            &harness.state,
        )),
        Err(error) => Ok(error_view(&error)),
    }
}

pub(super) fn creator_view(
    created: Option<&StreamChunk>,
    shaper: &ChunkShaper,
    state: &DecodeState,
) -> Value {
    json!({
        "watch": [
            state.received_finish_reason,
            state.intermittent_finish_reason,
            shaper.progress.sent_first_chunk,
            shaper.tool_call,
            shaper.chunks.len(),
        ],
        "chunk": created.map_or(Value::Null, summary_view),
    })
}

fn summary_view(chunk: &StreamChunk) -> Value {
    let choice = chunk.choices.first();
    json!([
        chunk.id,
        chunk.model,
        chunk.created,
        chunk.system_fingerprint,
        choice.and_then(|first| first.finish_reason.clone()),
        choice.and_then(|first| first.delta.content.clone()),
        choice.and_then(|first| first.delta.role.clone()),
        choice.map_or(Value::Null, |first| first
            .delta
            .function_call
            .as_ref()
            .map_or(Value::Null, function_view)),
        choice.map_or(Value::Null, |first| match first.delta.tool_calls.as_ref() {
            Some(JsonValue::Array(items)) => {
                Value::Array(items.iter().map(tool_call_view).collect())
            }
            Some(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Object(_),
            )
            | None => Value::Null,
        }),
        chunk.choices.len(),
        chunk.usage.as_ref().map_or(Value::Null, usage_view),
        object_value(chunk.provider_specific_fields.as_ref()),
    ])
}

fn function_view(value: &JsonValue) -> Value {
    match value {
        JsonValue::Object(fields) => json!([fields.get("name"), fields.get("arguments")]),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Value::Null,
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
    let function = fields.and_then(|found| found.get("function"));
    json!([
        fields.and_then(|found| found.get("id")),
        fields.and_then(|found| found.get("type")),
        function.map_or(Value::Null, function_view),
    ])
}
