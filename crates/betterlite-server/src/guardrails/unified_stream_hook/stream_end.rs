use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::unified_dispatch::stream_items::chunk_has_finish_reason;

use crate::guardrails::anthropic_translation::sse_events::{
    SseChunk, streaming_has_ended as anthropic_has_ended,
};
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::responses_translation::stream_scan::streaming_has_ended as responses_has_ended;
use crate::guardrails::unified_guardrail::translation::UnifiedTranslation;

// LiteLLM probes _check_streaming_has_ended with hasattr; a translation without it never ends the stream
pub trait StreamEndCheck {
    fn streaming_has_ended(&self, items: &[GuardrailValue]) -> bool;
}

impl<G, S, A, H> StreamEndCheck for UnifiedTranslation<G, S, A, H>
where
    G: GuardrailContract + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    fn streaming_has_ended(&self, items: &[GuardrailValue]) -> bool {
        match self {
            Self::Chat(_) => items.iter().any(chunk_has_finish_reason),
            // the Responses probe reads only the last event
            Self::Responses(_) => items
                .last()
                .and_then(|last| serde_json::to_value(last).ok())
                .is_some_and(|event| responses_has_ended(&[event])),
            Self::Anthropic(_) => {
                let chunks: Vec<SseChunk> = items.iter().map(anthropic_chunk).collect();
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

// An Anthropic stream relays raw SSE frames, which reach the hook as text
fn anthropic_chunk(item: &GuardrailValue) -> SseChunk {
    match item {
        GuardrailValue::Text(frame) => SseChunk::Bytes(frame.as_bytes().to_vec()),
        GuardrailValue::Object(_) => {
            if let Ok(JsonValue::Object(fields)) = serde_json::to_value(item) {
                SseChunk::Event(fields)
            } else {
                SseChunk::Other
            }
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_) => SseChunk::Other,
    }
}

mod tests;
