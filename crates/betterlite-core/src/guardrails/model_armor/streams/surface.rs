use super::chunk::Chunk;
use super::wire::StreamWire;

// Wire format of a buffered stream, which decides how it is assembled, scanned and refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    ChatCompletions,
    AnthropicMessages,
    Responses,
    OpaqueSse,
}

// LiteLLM's _classify_stream: raw frames are decoded before any event type is read.
#[must_use]
pub fn classify<W: StreamWire>(wire: &W, chunks: &[Chunk]) -> Surface {
    if chunks.iter().any(|chunk| matches!(chunk, Chunk::Bytes(_))) {
        return if wire.is_anthropic_sse(chunks) {
            Surface::AnthropicMessages
        } else {
            Surface::OpaqueSse
        };
    }
    if chunks
        .iter()
        .filter_map(Chunk::event_type)
        .any(|kind| kind.starts_with("response."))
    {
        Surface::Responses
    } else {
        Surface::ChatCompletions
    }
}

// LiteLLM's _is_terminal_error_stream: the refusal a preceding guardrail already produced.
#[must_use]
pub fn is_terminal_error<W: StreamWire>(wire: &W, chunks: &[Chunk]) -> bool {
    chunks
        .iter()
        .all(|chunk| chunk.event_type() == Some("error"))
        || wire.is_sse_error(chunks)
}
