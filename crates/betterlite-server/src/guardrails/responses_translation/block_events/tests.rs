#![cfg(test)]
use super::*;

#[test]
fn blocked_response_has_stamp_and_item() {
    let stamp = BlockStamp {
        response_id: "resp_fixed".into(),
        message_id: "msg_fixed".into(),
        created_at: 7,
    };
    let response = build_blocked_response("stop", "m", None, &stamp);
    assert_eq!(response["id"], "resp_fixed");
    assert_eq!(response["output"][0]["content"][0]["text"], "stop");
}

#[test]
fn standalone_block_emits_data_frames_and_sequence_numbers_advance() {
    let stamp = BlockStamp {
        response_id: "resp_fixed".into(),
        message_id: "msg_fixed".into(),
        created_at: 7,
    };
    let frames = build_block_sse_chunks("x", "m", None, false, None, &stamp).unwrap();
    assert!(!frames.is_empty());
    assert!(frames[0].starts_with(b"data: "));
    assert_eq!(
        next_stream_sequence_number(Some(&[json!({"sequence_number":4})])),
        5
    );
}
