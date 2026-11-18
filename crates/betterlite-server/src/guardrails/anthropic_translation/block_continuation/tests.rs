#![cfg(test)]
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse;
use std::collections::BTreeMap;

use super::*;

fn frame_text(chunk: &[u8]) -> String {
    std::str::from_utf8(chunk).unwrap().to_owned()
}

fn blocked<'a>(
    message: &'a str,
    original_response: Option<&'a OriginalResponse>,
) -> BlockedException<'a> {
    BlockedException {
        message,
        model: "claude-test",
        message_id: "msg_1",
        original_response,
    }
}

fn original_usage() -> OriginalResponse {
    OriginalResponse::Response(BTreeMap::from([(
        "usage".to_owned(),
        GuardrailValue::Object(BTreeMap::from([
            ("input_tokens".to_owned(), GuardrailValue::SignedInteger(10)),
            ("output_tokens".to_owned(), GuardrailValue::SignedInteger(2)),
        ])),
    )]))
}

#[test]
fn standalone_block_is_fake_anthropic_stream() {
    let frames = build_block_sse_chunks(&blocked("blocked", None), false, None).unwrap();
    assert!(frames[0].starts_with(b"event: message_start\n"));
    assert_eq!(frames.len(), 6);
    assert!(frames[4].starts_with(b"event: message_delta\n"));
    assert!(frames[5].starts_with(b"event: message_stop\n"));
    assert!(
        frames
            .iter()
            .skip(1)
            .all(|frame| !frame.starts_with(b"event: message_start\n"))
    );
}

#[test]
fn standalone_block_uses_original_usage() {
    let original = original_usage();
    let frames = build_block_sse_chunks(&blocked("blocked", Some(&original)), false, None).unwrap();
    let start = frame_text(&frames[0]);
    assert!(
        start.contains("\"usage\": {\"input_tokens\": 10, \"output_tokens\": 0}"),
        "{start}"
    );
    let delta = frame_text(&frames[4]);
    assert!(delta.contains("\"output_tokens\": 2"), "{delta}");
}

#[test]
fn continuation_closes_open_block_and_adds_a_text_block() {
    let previous = [SseChunk::Bytes(
        b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1}\n\n"
            .to_vec(),
    )];
    let frames =
        build_block_sse_chunks(&blocked("blocked, \u{1f5dd}", None), true, Some(&previous))
            .unwrap();
    assert_eq!(frames.len(), 6);
    assert_eq!(
        frame_text(&frames[0]),
        "event: content_block_stop\ndata: {\"type\": \"content_block_stop\", \"index\": 1}\n\n"
    );
    assert_eq!(
        frame_text(&frames[1]),
        "event: content_block_start\ndata: {\"type\": \"content_block_start\", \"index\": 2, \"content_block\": {\"type\": \"text\", \"text\": \"\"}}\n\n"
    );
    assert_eq!(
        frame_text(&frames[2]),
        "event: content_block_delta\ndata: {\"type\": \"content_block_delta\", \"index\": 2, \"delta\": {\"type\": \"text_delta\", \"text\": \"blocked, \\ud83d\\udd5d\"}}\n\n"
    );
    assert_eq!(
        frame_text(&frames[3]),
        "event: content_block_stop\ndata: {\"type\": \"content_block_stop\", \"index\": 2}\n\n"
    );
    assert_eq!(
        frame_text(&frames[4]),
        "event: message_delta\ndata: {\"type\": \"message_delta\", \"delta\": {\"stop_reason\": \"end_turn\", \"stop_sequence\": null}, \"usage\": {\"output_tokens\": 0}}\n\n"
    );
    assert_eq!(
        frame_text(&frames[5]),
        "event: message_stop\ndata: {\"type\": \"message_stop\"}\n\n"
    );
}

#[test]
fn continuation_without_open_block_starts_at_index_zero() {
    let frames = build_block_sse_chunks(&blocked("blocked", None), true, None).unwrap();
    assert_eq!(frames.len(), 5);
    assert_eq!(
        frame_text(&frames[0]),
        "event: content_block_start\ndata: {\"type\": \"content_block_start\", \"index\": 0, \"content_block\": {\"type\": \"text\", \"text\": \"\"}}\n\n"
    );
}

#[test]
fn continuation_uses_the_original_output_tokens() {
    let original = original_usage();
    let previous = [SseChunk::Bytes(
        b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0}\n\n"
            .to_vec(),
    )];
    let frames =
        build_block_sse_chunks(&blocked("blocked", Some(&original)), true, Some(&previous))
            .unwrap();
    let delta = frame_text(&frames[4]);
    assert!(delta.contains("\"output_tokens\": 2"), "{delta}");
}

#[test]
fn error_frames_match_shared_error_builder() {
    let frames = build_stream_error_items("blocked").unwrap();
    assert_eq!(frames.len(), 1);
    let frame = frame_text(&frames[0]);
    assert!(frame.starts_with("event: error\n"), "{frame}");
    assert!(frame.contains("\"guardrail_error\""), "{frame}");
}
