pub mod assemble_anthropic_sse_stream;
pub mod error;
pub mod error_frames;
pub mod is_anthropic_sse_stream;
pub mod is_raw_sse_stream;
pub mod is_sse_error_stream;
pub mod model_response_text;
pub mod sse_chunks_from_response;
pub mod sse_stream;

mod tests;

// The task's check selects these tests by a module name past the 30-character name limit, so they sit in the module itself, every item gated on cfg(test).
include!("anthropic_sse/tests/chunks_from_response.rs");
