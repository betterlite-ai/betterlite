use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::{ChatResponse, Response};
use betterlite_streaming::streamed_response_assembly::{Request, stream_chunk_builder};

use crate::guardrail_message_scope::{Object, StreamItem};

use super::anthropic_chunks::{ConvertedChunk, convert_anthropic_events};
use super::error::ResponseToolError;
use super::stream_services::StreamServices;
use super::stream_support::{join_raw_items, sse_events};

// LiteLLM's stream_chunk_builder over parsed OpenAI chunks; a text or empty response has nothing to check.
pub(crate) async fn assemble_openai_chunks(
    items: &[StreamItem],
    services: &StreamServices<'_>,
) -> Result<Option<Object>, ResponseToolError> {
    let mut chunks = Vec::with_capacity(items.len());
    for item in items {
        let StreamItem::Json(GuardrailValue::Object(fields)) = item else {
            // A chunk that is not a JSON object is outside the declared chunk type, so nothing assembles.
            return Ok(None);
        };
        chunks.push(chunk_record(fields)?);
    }
    if chunks.is_empty() {
        return Ok(None);
    }
    let request = Request {
        chunks: Some(chunks.as_slice()),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: services.costs,
        counter: services.counter,
        metadata: None,
        now: services.now,
    };
    let hooks = services.hooks();
    match stream_chunk_builder(&request, &hooks).await {
        Ok(Some(Response::Chat(chat))) => Ok(Some(response_object(&chat)?)),
        Ok(Some(Response::Text(_)) | None) => Ok(None),
        Err(error) => Err(ResponseToolError::Assembly(error)),
    }
}

// LiteLLM's assemble_anthropic_sse_stream: every failure during this assembly reads as no response to check.
pub(crate) async fn assemble_anthropic_stream(
    items: &[StreamItem],
    services: &StreamServices<'_>,
) -> Option<Object> {
    let raw = join_raw_items(items)?;
    let events = sse_events(&raw);
    let converted = convert_anthropic_events(&events, services.now)?;
    let mut chunks = Vec::with_capacity(converted.len());
    for chunk in &converted {
        chunks.push(converted_record(chunk)?);
    }
    if chunks.is_empty() {
        return None;
    }
    let request = Request {
        chunks: Some(chunks.as_slice()),
        messages: None,
        count_prompt_tokens: None,
        logging: None,
        costs: services.costs,
        counter: services.counter,
        metadata: None,
        now: services.now,
    };
    let hooks = services.hooks();
    match stream_chunk_builder(&request, &hooks).await {
        Ok(Some(Response::Chat(chat))) => response_object(&chat).ok(),
        Ok(Some(Response::Text(_)) | None) | Err(_) => None,
    }
}

fn chunk_record(fields: &Object) -> Result<Chunk, ResponseToolError> {
    let encoded = serde_json::to_string(fields)?;
    Ok(serde_json::from_str(&encoded)?)
}

fn converted_record(chunk: &ConvertedChunk<'_>) -> Option<Chunk> {
    let encoded = serde_json::to_string(chunk).ok()?;
    serde_json::from_str(&encoded).ok()
}

fn response_object(response: &ChatResponse) -> Result<Object, ResponseToolError> {
    let value = serde_json::to_value(response)?;
    Ok(serde_json::from_value::<Object>(value)?)
}
