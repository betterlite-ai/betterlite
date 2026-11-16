#![cfg(test)]

use rstest::rstest;

use super::is_anthropic_sse_stream;
use crate::anthropic_sse::tests::support::{
    GOOGLE_FRAME, MESSAGE_START, bytes_chunk, object_chunk, text_chunk,
};

#[rstest]
#[case::message_start("message_start")]
#[case::message_delta("message_delta")]
#[case::message_stop("message_stop")]
#[case::content_block_start("content_block_start")]
#[case::content_block_delta("content_block_delta")]
#[case::content_block_stop("content_block_stop")]
#[case::ping("ping")]
#[case::error("error")]
fn is_anthropic_sse_stream_recognizes_each_event_type(#[case] kind: &str) {
    let frame = format!("event: {kind}\ndata: {{\"type\": \"{kind}\"}}\n\n");
    assert!(is_anthropic_sse_stream(&[text_chunk(&frame)]));
}

#[test]
fn is_anthropic_sse_stream_reads_bytes_and_text_chunks_together() {
    let chunks = [bytes_chunk(MESSAGE_START.as_bytes()), object_chunk()];
    assert!(is_anthropic_sse_stream(&chunks));
}

#[test]
fn is_anthropic_sse_stream_is_false_for_a_google_stream() {
    assert!(!is_anthropic_sse_stream(&[text_chunk(GOOGLE_FRAME)]));
}

#[rstest]
#[case::done_marker("data: [DONE]\n\n")]
#[case::not_json("data: {oops\n\n")]
#[case::no_data_line("event: message_start\n\n")]
#[case::array_event("data: [1, 2]\n\n")]
#[case::type_is_not_text("data: {\"type\": 5}\n\n")]
#[case::no_type("data: {\"message\": {}}\n\n")]
#[case::unknown_type("data: {\"type\": \"response.created\"}\n\n")]
fn is_anthropic_sse_stream_is_false_without_an_event_type(#[case] frame: &str) {
    assert!(!is_anthropic_sse_stream(&[text_chunk(frame)]));
}

#[test]
fn is_anthropic_sse_stream_is_false_without_raw_chunks() {
    assert!(!is_anthropic_sse_stream(&[object_chunk()]));
    assert!(!is_anthropic_sse_stream(&[]));
}

#[test]
fn is_anthropic_sse_stream_is_false_for_bytes_that_are_not_utf8() {
    let mut frame = MESSAGE_START.as_bytes().to_vec();
    frame.push(0xFF);
    assert!(!is_anthropic_sse_stream(&[bytes_chunk(&frame)]));
}

#[test]
fn is_anthropic_sse_stream_holds_back_a_character_cut_off_at_the_end() {
    let mut frame = MESSAGE_START.as_bytes().to_vec();
    frame.push(0xC3);
    assert!(is_anthropic_sse_stream(&[bytes_chunk(&frame)]));
}

#[test]
fn is_anthropic_sse_stream_joins_a_character_split_across_chunks() {
    let frame = "data: {\"type\": \"ping\", \"note\": \"\u{e9}\"}\n\n".as_bytes();
    let split = frame
        .iter()
        .position(|byte| *byte == 0xC3)
        .unwrap()
        .saturating_add(1);
    let chunks = [bytes_chunk(&frame[..split]), bytes_chunk(&frame[split..])];
    assert!(is_anthropic_sse_stream(&chunks));
}
