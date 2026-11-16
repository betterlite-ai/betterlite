#![cfg(test)]

use crate::guardrail_base::failure::GuardrailFailure;

use super::{DEFAULT_MAX_STREAM_CHUNKS, StreamBufferLimits, buffer_overflow_failure};

#[test]
fn default_stream_buffer_limit_is_ten_thousand_chunks() {
    assert_eq!(DEFAULT_MAX_STREAM_CHUNKS, 10_000);
    assert_eq!(StreamBufferLimits::default().max_chunks, 10_000);
}

#[test]
fn stream_buffer_limits_keep_a_configured_limit() {
    assert_eq!(StreamBufferLimits { max_chunks: 3 }.max_chunks, 3);
}

#[test]
fn buffer_overflow_failure_names_the_limit_and_the_guardrail() {
    let failure = buffer_overflow_failure("tools-check", 25);
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
    assert_eq!(
        message,
        "Streamed response exceeded the 25 chunk buffer limit for tool permission checks, blocking it"
    );
    assert!(wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(!blocked_content);
}
