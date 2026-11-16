// LiteLLM's message for a raw stream that no Anthropic assembler could turn into a response to check
pub const UNPARSEABLE_STREAM_MESSAGE: &str = "Streamed response could not be verified for tool permissions (not a parseable Anthropic SSE stream), blocking it";

mod tests;
