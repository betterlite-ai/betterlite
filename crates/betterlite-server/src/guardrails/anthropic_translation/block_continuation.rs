use betterlite_core::api::messages_fake_stream::MessagesFakeStream;
use betterlite_guardrails::anthropic_sse::error_frames::anthropic_sse_error_frames;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::{
    BlockedUsage, OriginalResponse, blocked_response_usage,
};
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::sse_events::{SseChunk, content_block_state};
use frames::{Event, Stop, Text, Usage};

mod frames;

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
        return continuation_frames(blocked, usage, responses_so_far);
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
) -> Result<Vec<Vec<u8>>, AnthropicBlockError> {
    let (open_index, highest) = content_block_state(responses_so_far.unwrap_or_default());
    let index = highest.map_or(0, |seen| seen.saturating_add(1));
    let mut events = Vec::new();
    if let Some(open) = open_index {
        events.push(Event::ContentBlockStop { index: open });
    }
    events.extend([
        Event::ContentBlockStart {
            index,
            content_block: Text {
                r#type: "text",
                text: "",
            },
        },
        Event::ContentBlockDelta {
            index,
            delta: Text {
                r#type: "text_delta",
                text: blocked.message,
            },
        },
        Event::ContentBlockStop { index },
        Event::MessageDelta {
            delta: Stop {
                stop_reason: "end_turn",
                stop_sequence: None,
            },
            usage: Usage {
                output_tokens: usage.output_tokens,
            },
        },
        Event::MessageStop,
    ]);
    events.iter().map(frames::encode).collect()
}

pub fn build_stream_error_items(message: &str) -> Result<Vec<Vec<u8>>, AnthropicBlockError> {
    anthropic_sse_error_frames(message)
        .map_err(|source| AnthropicBlockError::ErrorFrames { source })
}

mod tests;
