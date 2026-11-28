use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::unified_dispatch::stream_items::chunk_has_finish_reason;

use crate::guardrails::anthropic_translation::sse_events::{
    SseChunk, stream_item_chunk, streaming_has_ended as anthropic_has_ended,
};
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::responses_translation::stream_scan::streaming_has_ended as responses_has_ended;
use crate::guardrails::unified_guardrail::translation::UnifiedTranslation;

// LiteLLM probes _check_streaming_has_ended with hasattr; a translation without it never ends the stream
pub trait StreamEndCheck {
    fn streaming_has_ended(&self, items: &[StreamItem]) -> bool;
}

impl<G, S, A, H> StreamEndCheck for UnifiedTranslation<G, S, A, H>
where
    G: GuardrailContract + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    fn streaming_has_ended(&self, items: &[StreamItem]) -> bool {
        match self {
            Self::Chat(_) => items.iter().any(|item| match item {
                StreamItem::Json(value) => chunk_has_finish_reason(value),
                StreamItem::Bytes(_) => false,
            }),
            // the Responses probe reads only the last event
            Self::Responses(_) => items
                .last()
                .and_then(|last| match last {
                    StreamItem::Json(value) => serde_json::to_value(value).ok(),
                    StreamItem::Bytes(_) => None,
                })
                .is_some_and(|event| responses_has_ended(&[event])),
            Self::Anthropic(_) => {
                let chunks: Vec<SseChunk> = items
                    .iter()
                    .map(|item| stream_item_chunk(item).unwrap_or(SseChunk::Other))
                    .collect();
                anthropic_has_ended(&chunks)
            }
            Self::Completion(_)
            | Self::Embeddings(_)
            | Self::Image(_)
            | Self::Speech(_)
            | Self::Transcription(_)
            | Self::Video(_)
            | Self::Endpoint(_)
            | Self::LlmRoute(_)
            | Self::A2a(_)
            | Self::Rerank(_)
            | Self::GoogleGenAi(_)
            | Self::Mcp(_) => false,
        }
    }
}

mod tests;
