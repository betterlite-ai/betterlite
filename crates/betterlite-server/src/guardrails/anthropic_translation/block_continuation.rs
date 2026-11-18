use betterlite_core::api::messages_fake_stream::MessagesFakeStream;
use betterlite_guardrails::anthropic_sse::error_frames::anthropic_sse_error_frames;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::{
    BlockedUsage, OriginalResponse, blocked_response_usage,
};
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

use super::sse_events::{SseChunk, content_block_state};

#[derive(Debug, thiserror::Error)]
pub enum AnthropicBlockError {
    #[error(transparent)]
    FakeStream(#[from] betterlite_core::api::messages_fake_stream::MessagesFakeStreamError),
    #[error("Anthropic stream event serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Anthropic stream error serialization failed: {source}")]
    ErrorFrames {
        #[source]
        source: betterlite_guardrails::anthropic_sse::error::AnthropicSseError,
    },
}

#[derive(Debug)]
pub struct BlockedException<'blocked> {
    pub message: &'blocked str,
    pub model: &'blocked str,
    pub message_id: &'blocked str,
    pub original_response: Option<&'blocked OriginalResponse>,
}

#[derive(Serialize)]
struct BlockedMessage<'message> {
    id: &'message str,
    r#type: &'message str,
    role: &'message str,
    content: [BlockedText<'message>; 1],
    model: &'message str,
    stop_reason: &'message str,
    usage: BlockedUsage,
}

#[derive(Serialize)]
struct BlockedText<'text> {
    r#type: &'text str,
    text: &'text str,
}

pub fn build_block_sse_chunks(
    blocked: &BlockedException<'_>,
    stream_started: bool,
    responses_so_far: Option<&[SseChunk]>,
) -> Result<Vec<Vec<u8>>, AnthropicBlockError> {
    let usage = blocked_response_usage(
        blocked
            .original_response
            .unwrap_or(&OriginalResponse::Absent),
    );
    if stream_started {
        return Ok(continuation_frames(blocked, usage, responses_so_far));
    }
    let payload = BlockedMessage {
        id: blocked.message_id,
        r#type: "message",
        role: "assistant",
        content: [BlockedText {
            r#type: "text",
            text: blocked.message,
        }],
        model: blocked.model,
        stop_reason: "end_turn",
        usage,
    };
    let response: Json = serde_json::from_value(serde_json::to_value(payload)?)?;
    Ok(MessagesFakeStream::new(&response)?.collect())
}

fn continuation_frames(
    blocked: &BlockedException<'_>,
    usage: BlockedUsage,
    responses_so_far: Option<&[SseChunk]>,
) -> Vec<Vec<u8>> {
    let (open_index, highest) = content_block_state(responses_so_far.unwrap_or_default());
    let new_index = highest.map_or(0, |index| index.saturating_add(1));
    let mut frames = Vec::new();
    if let Some(index) = open_index {
        frames.push(block_stop_frame(index));
    }
    frames.push(block_start_frame(new_index));
    frames.push(block_delta_frame(new_index, blocked.message));
    frames.push(block_stop_frame(new_index));
    frames.push(message_delta_frame(usage.output_tokens));
    frames.push(message_stop_frame());
    frames
}

fn block_stop_frame(index: i64) -> Vec<u8> {
    let event = Json::Object(IndexMap::from([
        (
            "type".to_owned(),
            Json::Text("content_block_stop".to_owned()),
        ),
        ("index".to_owned(), Json::Number(Number::from(index))),
    ]));
    let data = dumps(&event);
    format!("event: content_block_stop\ndata: {data}\n\n").into_bytes()
}

fn block_start_frame(index: i64) -> Vec<u8> {
    let event = Json::Object(IndexMap::from([
        (
            "type".to_owned(),
            Json::Text("content_block_start".to_owned()),
        ),
        ("index".to_owned(), Json::Number(Number::from(index))),
        (
            "content_block".to_owned(),
            Json::Object(IndexMap::from([
                ("type".to_owned(), Json::Text("text".to_owned())),
                ("text".to_owned(), Json::Text(String::new())),
            ])),
        ),
    ]));
    let data = dumps(&event);
    format!("event: content_block_start\ndata: {data}\n\n").into_bytes()
}

fn block_delta_frame(index: i64, text: &str) -> Vec<u8> {
    let event = Json::Object(IndexMap::from([
        (
            "type".to_owned(),
            Json::Text("content_block_delta".to_owned()),
        ),
        ("index".to_owned(), Json::Number(Number::from(index))),
        (
            "delta".to_owned(),
            Json::Object(IndexMap::from([
                ("type".to_owned(), Json::Text("text_delta".to_owned())),
                ("text".to_owned(), Json::Text(text.to_owned())),
            ])),
        ),
    ]));
    let data = dumps(&event);
    format!("event: content_block_delta\ndata: {data}\n\n").into_bytes()
}

fn message_delta_frame(output_tokens: i64) -> Vec<u8> {
    let event = Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("message_delta".to_owned())),
        (
            "delta".to_owned(),
            Json::Object(IndexMap::from([
                ("stop_reason".to_owned(), Json::Text("end_turn".to_owned())),
                ("stop_sequence".to_owned(), Json::Null),
            ])),
        ),
        (
            "usage".to_owned(),
            Json::Object(IndexMap::from([(
                "output_tokens".to_owned(),
                Json::Number(Number::from(output_tokens)),
            )])),
        ),
    ]));
    let data = dumps(&event);
    format!("event: message_delta\ndata: {data}\n\n").into_bytes()
}

fn message_stop_frame() -> Vec<u8> {
    let event = Json::Object(IndexMap::from([(
        "type".to_owned(),
        Json::Text("message_stop".to_owned()),
    )]));
    let data = dumps(&event);
    format!("event: message_stop\ndata: {data}\n\n").into_bytes()
}

pub fn build_stream_error_items(message: &str) -> Result<Vec<Vec<u8>>, AnthropicBlockError> {
    anthropic_sse_error_frames(message)
        .map_err(|source| AnthropicBlockError::ErrorFrames { source })
}

#[cfg(test)]
mod tests;
