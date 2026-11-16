#![cfg(test)]

use crate::guardrail_base::failure::GuardrailFailure;

use super::{UNPARSEABLE_STREAM_MESSAGE, unparseable_stream_failure};

#[test]
fn unparseable_stream_message_matches_litellm() {
    assert_eq!(
        UNPARSEABLE_STREAM_MESSAGE,
        "Streamed response could not be verified for tool permissions (not a parseable Anthropic SSE stream), blocking it"
    );
}

#[test]
fn unparseable_stream_failure_names_the_guardrail_and_blocks() {
    let failure = unparseable_stream_failure("tools-check");
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        wrap_default_message,
        status_code,
        blocked_content,
    } = failure
    else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(guardrail_name.as_deref(), Some("tools-check"));
    assert_eq!(message, UNPARSEABLE_STREAM_MESSAGE);
    assert!(wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(!blocked_content);
}
