use betterlite_models::decisions::HiddenParams;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::chat_choices::stream::{StreamError, chunk_parser as openai_chunk_parser};
use crate::chat_choices::wire::StreamReply;
use crate::fireworks_ai_chat::hidden::extract_fireworks_hidden_params;

#[derive(Debug, Clone, Serialize)]
pub struct FireworksChunk {
    #[serde(flatten)]
    pub reply: StreamReply,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<HiddenParams>,
}

// FireworksAIChatCompletionStreamingHandler.chunk_parser: B504's chunk, then the Fireworks fields merged over it
pub fn chunk_parser(
    chunk: &Value,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<FireworksChunk, StreamError> {
    let reply = openai_chunk_parser(chunk, identifier, clock)?;
    let fields = extract_fireworks_hidden_params(chunk);
    Ok(FireworksChunk {
        reply,
        provider_specific_fields: (!fields.is_empty()).then_some(fields),
    })
}
