#![cfg(test)]

use rstest::rstest;

use super::is_sse_error_stream;
use crate::guardrails::anthropic_sse::tests::support::{
    CHAT_ERROR, ERROR_EVENT, MESSAGE_START, bytes_chunk, object_chunk, text_chunk,
};

#[rstest]
#[case::anthropic_error_event(ERROR_EVENT)]
#[case::chat_error_payload(CHAT_ERROR)]
#[case::error_type_without_a_body("data: {\"type\": \"error\"}\n\n")]
fn is_sse_error_stream_is_true_for_error_frames(#[case] frame: &str) {
    assert!(is_sse_error_stream(&[text_chunk(frame)]));
}

#[test]
fn is_sse_error_stream_is_true_for_both_wire_forms_together() {
    let chunks = [text_chunk(ERROR_EVENT), bytes_chunk(CHAT_ERROR.as_bytes())];
    assert!(is_sse_error_stream(&chunks));
}

#[test]
fn is_sse_error_stream_fails_closed_on_a_stream_with_real_content() {
    let chunks = [text_chunk(MESSAGE_START), text_chunk(ERROR_EVENT)];
    assert!(!is_sse_error_stream(&chunks));
}

#[test]
fn is_sse_error_stream_fails_closed_when_a_typed_chunk_is_mixed_in() {
    let chunks = [object_chunk(), text_chunk(ERROR_EVENT)];
    assert!(!is_sse_error_stream(&chunks));
}

#[rstest]
#[case::error_that_is_text("data: {\"error\": \"blocked\"}\n\n")]
#[case::array_event("data: [1]\n\n")]
#[case::no_events("data: [DONE]\n\n")]
fn is_sse_error_stream_is_false_for_frames_that_are_not_errors(#[case] frame: &str) {
    assert!(!is_sse_error_stream(&[text_chunk(frame)]));
}

#[test]
fn is_sse_error_stream_is_false_for_no_chunks() {
    assert!(!is_sse_error_stream(&[]));
}

#[test]
fn is_sse_error_stream_is_false_for_bytes_that_are_not_utf8() {
    let mut frame = ERROR_EVENT.as_bytes().to_vec();
    frame.push(0xFF);
    assert!(!is_sse_error_stream(&[bytes_chunk(&frame)]));
}
