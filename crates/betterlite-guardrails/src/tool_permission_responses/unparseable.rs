use crate::guardrail_base::failure::GuardrailFailure;

// LiteLLM's message for a raw stream that no Anthropic assembler could turn into a response to check
pub const UNPARSEABLE_STREAM_MESSAGE: &str = "Streamed response could not be verified for tool permissions (not a parseable Anthropic SSE stream), blocking it";

// An allegedly raw SSE stream that could not be assembled is blocked rather than relayed unchecked.
#[must_use]
pub fn unparseable_stream_failure(guardrail_name: &str) -> GuardrailFailure {
    GuardrailFailure::GuardrailRaised {
        guardrail_name: Some(guardrail_name.to_owned()),
        message: UNPARSEABLE_STREAM_MESSAGE.to_owned(),
        wrap_default_message: true,
        status_code: 400,
        blocked_content: false,
    }
}

mod tests;
