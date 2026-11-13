use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::{
    OriginalResponse, blocked_chat_stream_usage,
};

use super::blocked_identity::{FreshIdentity, blocked_stream_identity};
use super::error::BlockFramingError;
use super::sse_chunk::{
    BlockedChunk, BlockedChunkChoice, BlockedChunkDelta, BlockedChunkUsage, chat_sse_chunk,
};

#[derive(Debug, Clone, PartialEq)]
pub struct BlockedResponse {
    pub message: String,
    pub model: String,
    pub original_response: OriginalResponse,
}

// A pre-stream block opens a completion with a role delta; a mid-stream block continues the one the client already has, content only.
pub fn build_block_sse_chunks(
    exception: &BlockedResponse,
    stream_started: bool,
    responses_so_far: &[StreamItem],
    fresh: &FreshIdentity,
) -> Result<[Vec<u8>; 2], BlockFramingError> {
    let identity = blocked_stream_identity(&exception.model, responses_so_far, fresh);
    let (prompt_tokens, completion_tokens) =
        blocked_chat_stream_usage(&exception.original_response);
    let message_chunk = BlockedChunk {
        id: &identity.id,
        object: "chat.completion.chunk",
        created: identity.created,
        model: &identity.model,
        choices: [BlockedChunkChoice {
            index: 0,
            delta: BlockedChunkDelta {
                role: (!stream_started).then_some("assistant"),
                content: Some(&exception.message),
            },
            finish_reason: None,
        }],
        usage: None,
    };
    let message_frame = chat_sse_chunk(&message_chunk)?;
    let final_chunk = BlockedChunk {
        choices: [BlockedChunkChoice {
            index: 0,
            delta: BlockedChunkDelta::default(),
            finish_reason: Some("content_filter"),
        }],
        usage: Some(BlockedChunkUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
        }),
        ..message_chunk
    };
    Ok([message_frame, chat_sse_chunk(&final_chunk)?])
}
