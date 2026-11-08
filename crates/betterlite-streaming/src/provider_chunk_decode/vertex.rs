use uuid::Uuid;

use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, as_array, as_object, json_text,
};
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChoice, StreamChunk, StreamDelta,
};
use crate::stream_chunk_shaping::tokens::mapped_finish_reason;

// A vertex ai chunk arrives as the proto object's JSON, candidates and all.
pub fn handle_vertex_chunk(
    chunk: &ProviderChunk,
    state: &mut DecodeState,
    decoded: &mut DecodedChunk,
) -> Result<(), ProviderChunkError> {
    if let ProviderChunk::Text(text) = chunk {
        decoded.text.clone_from(text);
        return Ok(());
    }
    let value = chunk.json()?;
    let Some(fields) = as_object(value) else {
        return Err(ProviderChunkError::UnexpectedChunk {
            expected: "a vertex ai chunk",
        });
    };
    let Some(listed) = fields.get("candidates") else {
        decoded.text = json_text(value);
        return Ok(());
    };
    let candidate = as_array(listed)
        .and_then(|items| items.first())
        .and_then(as_object)
        .ok_or(ProviderChunkError::MissingField {
            field: "candidates",
        })?;
    let finish_name = candidate.get("finish_reason").and_then(JsonValue::as_str);
    let parts = candidate
        .get("content")
        .and_then(as_object)
        .and_then(|content| content.get("parts"))
        .and_then(as_array);
    fill_vertex(text_of(parts), parts, candidate, decoded)?;
    if let Some(name) = finish_name.filter(|name| *name != "FINISH_REASON_UNSPECIFIED") {
        state.received_finish_reason = Some(mapped_finish_reason(name).to_owned());
    }
    Ok(())
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
    as_object(parts.first()?)?
        .get("function_call")
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
    let plain = serde_json::to_value(&args).map_err(ProviderChunkError::from)?;
    let arguments = serde_json::to_string(&plain).map_err(ProviderChunkError::from)?;
    let mut function = JsonObject::new();
    function.insert("arguments".to_owned(), JsonValue::String(arguments));
    function.insert("name".to_owned(), JsonValue::String(name.to_owned()));
    let mut entry = JsonObject::new();
    entry.insert(
        "id".to_owned(),
        JsonValue::String(format!("call_{}", Uuid::new_v4())),
    );
    entry.insert("function".to_owned(), JsonValue::Object(function));
    entry.insert("type".to_owned(), JsonValue::String("function".to_owned()));
    Ok(JsonValue::Object(entry))
}

fn vertex_tool_chunk(calls: &JsonValue) -> StreamChunk {
    StreamChunk {
        id: format!("chatcmpl-{}", Uuid::new_v4()),
        object: "chat.completion.chunk".to_owned(),
        created: 0,
        model: String::new(),
        system_fingerprint: None,
        choices: vec![StreamChoice {
            index: 0,
            delta: StreamDelta {
                content: None,
                role: None,
                function_call: None,
                tool_calls: Some(calls.clone()),
                audio: None,
                images: None,
                annotations: None,
                reasoning_content: None,
                thinking_blocks: None,
                provider_specific_fields: None,
                extra: JsonObject::new(),
            },
            finish_reason: None,
            logprobs: None,
            extra: JsonObject::new(),
        }],
        usage: None,
        provider_specific_fields: None,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    }
}
