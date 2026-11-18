#![cfg(test)]
use super::*;
use serde_json::{Value, json};

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
fn blocked_response_keeps_original_usage_counts() {
    let stamp = BlockStamp {
        response_id: "resp_fixed".into(),
        message_id: "msg_fixed".into(),
        created_at: 7,
    };
    let original = json!({"usage":{"input_tokens":8,"output_tokens":2,"total_tokens":10}});
    let response = build_blocked_response("stop", "m", Some(&original), &stamp);
    assert_eq!(response["usage"]["input_tokens"], 8);
    assert_eq!(response["usage"]["output_tokens"], 2);
    assert_eq!(response["usage"]["total_tokens"], 10);
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

#[test]
fn standalone_keeps_non_ascii_message_in_delta() {
    let stamp = BlockStamp {
        response_id: "resp_fixed".into(),
        message_id: "msg_fixed".into(),
        created_at: 7,
    };
    let frames = build_block_sse_chunks("é💥", "m", None, false, None, &stamp).unwrap();
    let events: Vec<Value> = frames
        .iter()
        .map(|frame| serde_json::from_slice(&frame[6..frame.len() - 2]).unwrap())
        .collect();
    let delta = events
        .iter()
        .find(|event| event["type"] == "response.output_text.delta")
        .unwrap();
    assert_eq!(delta["delta"], "é💥");
}

#[test]
fn continuation_closes_open_function_call_incomplete_before_block_item() {
    let stamp = BlockStamp {
        response_id: "resp_fixed".into(),
        message_id: "msg_fixed".into(),
        created_at: 7,
    };
    let events = [
        json!({"type":"response.created","response":{"id":"resp_stream","model":"stream-model"}}),
        json!({"type":"response.output_item.added","output_index":4,"item":{"type":"function_call","id":"call_1","call_id":"id","name":"tool","arguments":"{}"}}),
    ];
    let frames = build_block_sse_chunks("blocked", "m", None, true, Some(&events), &stamp).unwrap();
    let close: Value = serde_json::from_slice(&frames[0][6..frames[0].len() - 2]).unwrap();
    assert_eq!(close["type"], "response.output_item.done");
    assert_eq!(close["item"]["status"], "incomplete");
    let completed: Value = serde_json::from_slice(&frames.last().unwrap()[6..]).unwrap();
    assert_eq!(completed["response"]["id"], "resp_stream");
    assert_eq!(completed["response"]["model"], "stream-model");
}

#[test]
fn continuation_identity_skips_empty_fields_and_advances_output_index() {
    let stamp = BlockStamp {
        response_id: "resp_fallback".into(),
        message_id: "msg_fixed".into(),
        created_at: 7,
    };
    let events = [
        json!({"response":{"id":"","model":""},"output_index":1}),
        json!({"response":{"id":"resp_stream"},"output_index":7}),
        json!({"response":{"model":"model_stream"},"output_index":3}),
    ];
    assert_eq!(
        continuation_identity("given-model", Some(&events), &stamp),
        ("resp_stream".into(), "model_stream".into(), 8)
    );
}

#[test]
fn stream_error_has_guardrail_code_and_next_sequence() {
    let events = [json!({"sequence_number":5}), json!({"type":"event"})];
    let errors = build_stream_error_items(403, "blocked", Some(&events));
    assert_eq!(errors[0].error.kind, "guardrail_error");
    assert_eq!(errors[0].error.code, "403");
    assert_eq!(errors[0].sequence_number, 6);
}

#[test]
fn sequence_number_skips_missing_and_defaults_to_zero() {
    assert_eq!(next_stream_sequence_number(None), 0);
    assert_eq!(
        next_stream_sequence_number(Some(&[
            json!({"sequence_number":8}),
            json!({"type":"no_sequence"})
        ])),
        9
    );
}
