use crate::guardrail_base::failure::GuardrailFailure;

// LiteLLM buffers a streamed response with no bound at all; a public gateway needs a per-request ceiling on untrusted input.
pub const DEFAULT_MAX_STREAM_CHUNKS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBufferLimits {
    pub max_chunks: usize,
}

impl Default for StreamBufferLimits {
    fn default() -> Self {
        Self {
            max_chunks: DEFAULT_MAX_STREAM_CHUNKS,
        }
    }
}

// A buffered stream past the ceiling is refused before another chunk joins it, the way a blocked stream is refused.
#[must_use]
pub fn buffer_overflow_failure(guardrail_name: &str, limit: usize) -> GuardrailFailure {
    GuardrailFailure::GuardrailRaised {
        guardrail_name: Some(guardrail_name.to_owned()),
        message: format!(
            "Streamed response exceeded the {limit} chunk buffer limit for tool permission checks, blocking it"
        ),
        wrap_default_message: true,
        status_code: 400,
        blocked_content: false,
    }
}

mod tests;
