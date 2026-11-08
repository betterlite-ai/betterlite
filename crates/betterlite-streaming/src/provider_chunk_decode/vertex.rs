use uuid::Uuid;

use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_array, as_object, at, first, json_text, object,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChoice, StreamChunk};
use crate::stream_chunk_shaping::tokens::mapped_finish_reason;

// A vertex ai chunk arrives as the proto object's JSON, candidates and all.
pub fn handle_vertex_chunk(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
) -> Result<DecodedChunk, ProviderChunkError> {
    let mut decoded = DecodedChunk::default();
    if let ProviderChunk::Text(text) = chunk {
        decoded.text.clone_from(text);
        return Ok(decoded);
    }
    let value = chunk.json()?;
    let Some(fields) = as_object(value) else {
        return Err(ProviderChunkError::UnexpectedChunk {
            expected: "a vertex ai chunk",
        });
    };
    let Some(listed) = fields.get("candidates") else {
        decoded.text = json_text(value);
        return Ok(decoded);
    };
    let candidate = first(listed)
        .and_then(as_object)
        .ok_or(ProviderChunkError::MissingField {
            field: "candidates",
        })?;
    let finish_name = candidate.get("finish_reason").and_then(JsonValue::as_str);
    let parts = candidate
        .get("content")
        .and_then(|content| at(content, "parts"))
        .and_then(as_array);
    fill_vertex(text_of(parts), parts, candidate, &mut decoded)?;
    if let Some(name) = finish_name.filter(|name| *name != "FINISH_REASON_UNSPECIFIED") {
        state.received_finish_reason = Some(mapped_finish_reason(name).to_owned());
    }
    Ok(decoded)
}

fn fill_vertex(
    chunk_text: Option<String>,
    parts: Option<&[JsonValue]>,
    candidate: &JsonObject,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    if let Some(text) = chunk_text {
        decoded.text = text;
        return Ok(());
    }
    if let Some(call) = parts.and_then(vertex_tool_call) {
        let calls = JsonValue::Array(vec![vertex_tool_call_value(&call)?]);
        decoded.tool_calls = Some(calls.clone());
        decoded.original = Some(vertex_tool_chunk(&calls));
        return Ok(());
    }
    if candidate.get("finish_reason").and_then(JsonValue::as_str) == Some("SAFETY") {
        return Err(ProviderChunkError::VertexBlocked);
    }
    Ok(())
}

fn text_of(parts: Option<&[JsonValue]>) -> Option<String> {
    let mut text = String::new();
    let mut found = false;
    for part in parts? {
        let fragment = as_object(part)
            .and_then(|fields| fields.get("text"))
            .and_then(JsonValue::as_str);
        if let Some(text_part) = fragment {
            text.push_str(text_part);
            found = true;
        }
    }
    found.then_some(text)
}

fn vertex_tool_call(parts: &[JsonValue]) -> Option<JsonObject> {
    at(parts.first()?, "function_call")
        .and_then(as_object)
        .cloned()
}

fn vertex_tool_call_value(call: &JsonObject) -> Result<JsonValue, ProviderChunkError> {
    let name = call
        .get("name")
        .and_then(JsonValue::as_str)
        .ok_or(ProviderChunkError::MissingField { field: "name" })?;
    let args = call
        .get("args")
        .cloned()
        .unwrap_or_else(|| JsonValue::Object(JsonObject::new()));
    let arguments = serde_json::to_string(&args).map_err(ProviderChunkError::from)?;
    let function = object(vec![
        ("arguments", JsonValue::String(arguments)),
        ("name", JsonValue::String(name.to_owned())),
    ]);
    Ok(object(vec![
        ("id", JsonValue::String(format!("call_{}", Uuid::new_v4()))),
        ("function", function),
        ("type", JsonValue::String("function".to_owned())),
    ]))
}

fn vertex_tool_chunk(calls: &JsonValue) -> StreamChunk {
    let mut chunk = StreamChunk::default();
    fill_tool_chunk(&mut chunk, calls);
    chunk
}

fn fill_tool_chunk(chunk: &mut StreamChunk, calls: &JsonValue) {
    let mut choice = StreamChoice::default();
    choice.delta.tool_calls = Some(calls.clone());
    chunk.id = format!("chatcmpl-{}", Uuid::new_v4());
    chunk.choices = vec![choice];
}
