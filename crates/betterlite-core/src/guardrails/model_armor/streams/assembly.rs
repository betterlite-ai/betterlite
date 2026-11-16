use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::chunk::Chunk;
use super::surface::Surface;
use super::wire::StreamWire;

const TERMINAL_EVENTS: [&str; 3] = [
    "response.completed",
    "response.incomplete",
    "response.failed",
];

// A buffered stream's assembled body. Only a ModelResponse keeps the choices a sanitized rewrite replaces.
#[derive(Debug, Clone, PartialEq)]
pub enum Assembly {
    Model(GuardrailValue),
    Completion(GuardrailValue),
    Responses(GuardrailValue),
}

// LiteLLM's _assemble_stream: one builder per wire format, and none for frames it cannot read.
pub async fn assemble<W: StreamWire>(
    wire: &W,
    surface: Surface,
    chunks: &[Chunk],
) -> Option<Assembly> {
    match surface {
        Surface::AnthropicMessages => wire.assemble_anthropic(chunks).await,
        Surface::Responses => terminal_body(chunks).map(Assembly::Responses),
        Surface::OpaqueSse => None,
        Surface::ChatCompletions => wire.assemble_chat(chunks).await,
    }
}

// LiteLLM's _final_responses_api_response: a stream cut short before its terminal event reads as unassembled.
#[must_use]
pub fn terminal_body(chunks: &[Chunk]) -> Option<GuardrailValue> {
    chunks.iter().rev().find_map(|chunk| {
        let kind = chunk.event_type()?;
        if !TERMINAL_EVENTS.contains(&kind) {
            return None;
        }
        chunk
            .payload_field("response")
            .filter(|body| matches!(body, GuardrailValue::Object(_)))
            .cloned()
    })
}
