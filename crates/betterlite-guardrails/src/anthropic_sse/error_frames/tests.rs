#![cfg(test)]

use super::anthropic_sse_error_frames;

#[test]
fn error_frames_are_one_anthropic_error_event() {
    let frames = anthropic_sse_error_frames("blocked by policy").unwrap();
    let expected = b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"guardrail_error\",\"message\":\"blocked by policy\"}}\n\n";
    assert_eq!(frames, [expected.to_vec()]);
}

#[test]
fn error_frames_escape_a_message_that_would_break_the_frame() {
    let frames = anthropic_sse_error_frames("say \"no\"\n\ndata: {}").unwrap();
    let expected = b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"guardrail_error\",\"message\":\"say \\\"no\\\"\\n\\ndata: {}\"}}\n\n";
    assert_eq!(frames, [expected.to_vec()]);
}
