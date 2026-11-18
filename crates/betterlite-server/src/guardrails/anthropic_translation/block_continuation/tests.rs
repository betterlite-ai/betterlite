#![cfg(test)]
use super::*;

#[test]
fn standalone_block_is_fake_anthropic_stream() {
    let chunks = build_block_sse_chunks("blocked", "m", None, false, None, "msg_1").unwrap();
    assert!(
        chunks
            .first()
            .unwrap()
            .starts_with(b"event: message_start\n")
    );
}

#[test]
fn continuation_closes_previous_block_without_second_start() {
    let previous = [SseChunk::Event(
        serde_json::from_value(serde_json::json!({"type":"content_block_start","index":0}))
            .unwrap(),
    )];
    let chunks =
        build_block_sse_chunks("blocked", "m", None, true, Some(&previous), "msg_1").unwrap();
    assert!(
        chunks
            .iter()
            .all(|chunk| !chunk.starts_with(b"event: message_start\n"))
    );
    assert_eq!(chunks.len(), 6);
}

#[test]
fn error_frames_match_shared_error_builder() {
    assert!(build_stream_error_items("blocked").unwrap()[0].starts_with(b"event: error\n"));
}
