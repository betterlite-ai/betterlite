use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Json(GuardrailValue),
    Bytes(Vec<u8>),
}

// LiteLLM's is_raw_sse_stream: a chunk that is not a parsed JSON object means the stream is replayed frames.
#[must_use]
pub fn is_raw_sse_stream(chunks: &[StreamChunk]) -> bool {
    chunks.iter().any(|chunk| {
        let StreamChunk::Json(value) = chunk else {
            return true;
        };
        matches!(value, GuardrailValue::Text(_))
    })
}
