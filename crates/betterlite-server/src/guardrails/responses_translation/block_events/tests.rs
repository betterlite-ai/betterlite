#![cfg(test)]
use serde_json::{Value, json};

use super::*;

fn stamp() -> BlockStamp {
    BlockStamp {
        response_id: "resp_fixed".to_owned(),
        message_id: "msg_fixed".to_owned(),
        created_at: 7,
    }
}

fn frame_value(frame: &[u8]) -> Value {
    let text = std::str::from_utf8(frame).unwrap();
    serde_json::from_str(text.strip_prefix("data: ").unwrap().trim()).unwrap()
}

fn original_usage() -> Value {
    json!({
        "usage": {
            "input_tokens": 8,
            "output_tokens": 2,
            "total_tokens": 10,
            "input_tokens_details": {"cached_tokens": 3},
        }
    })
}

#[test]
fn blocked_response_has_stamp_and_item() {
    let response = build_blocked_response("stop", "m", None, &stamp());
    assert_eq!(response["id"], "resp_fixed");
    assert_eq!(response["object"], "response");
    assert_eq!(response["created_at"], 7);
    assert_eq!(response["model"], "m");
    assert_eq!(response["status"], "completed");
    assert_eq!(response["output"][0]["type"], "message");
    assert_eq!(response["output"][0]["id"], "msg_fixed");
    assert_eq!(response["output"][0]["status"], "completed");
    assert_eq!(response["output"][0]["role"], "assistant");
    assert_eq!(response["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(response["output"][0]["content"][0]["text"], "stop");
    assert_eq!(
        response["output"][0]["content"][0]["annotations"],
        json!([])
    );
}

#[test]
fn blocked_response_keeps_original_usage_counts() {
    let original = original_usage();
    let response = build_blocked_response("stop", "m", Some(&original), &stamp());
    assert_eq!(response["usage"]["input_tokens"], 8);
    assert_eq!(response["usage"]["output_tokens"], 2);
    assert_eq!(response["usage"]["total_tokens"], 10);
    assert_eq!(
        response["usage"]["input_tokens_details"]["cached_tokens"],
        3
    );
}

#[test]
fn blocked_response_without_original_usage_counts_zero() {
    let response = build_blocked_response("stop", "m", None, &stamp());
    assert_eq!(response["usage"]["input_tokens"], 0);
    assert_eq!(response["usage"]["output_tokens"], 0);
    assert_eq!(response["usage"]["total_tokens"], 0);
}

#[test]
fn standalone_block_emits_data_frames_and_sequence_numbers_advance() {
    let frames = build_block_sse_chunks("x", "m", None, false, None, &stamp()).unwrap();
    assert!(!frames.is_empty());
    assert!(frames.iter().all(|frame| frame.starts_with(b"data: ")));
    assert!(frames.iter().all(|frame| frame.ends_with(b"\n\n")));
    let done = frames
        .iter()
        .map(|frame| frame_value(frame))
        .find(|event| event["type"] == "response.output_item.done")
        .unwrap();
    assert_eq!(done["sequence_number"], 1);
    assert_eq!(
        next_stream_sequence_number(Some(&[json!({"sequence_number":4})])),
        5
    );
}

#[test]
fn standalone_block_starts_with_created_and_ends_completed() {
    let frames = build_block_sse_chunks("x", "m", None, false, None, &stamp()).unwrap();
    let events: Vec<Value> = frames.iter().map(|frame| frame_value(frame)).collect();
    assert_eq!(events[0]["type"], "response.created");
    assert_eq!(events[1]["type"], "response.in_progress");
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    assert_eq!(events.last().unwrap()["response"]["id"], "resp_fixed");
    assert_eq!(events.last().unwrap()["response"]["status"], "completed");
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["content"][0]["text"],
        "x"
    );
}

#[test]
fn standalone_frames_omit_null_fields() {
    let frames = build_block_sse_chunks("x", "m", None, false, None, &stamp()).unwrap();
    for frame in &frames {
        let text = std::str::from_utf8(frame).unwrap();
        assert!(!text.contains("null"), "{text}");
    }
}

#[test]
fn standalone_keeps_non_ascii_message_in_delta() {
    let frames = build_block_sse_chunks("é💥", "m", None, false, None, &stamp()).unwrap();
    let delta = frames
        .iter()
        .map(|frame| frame_value(frame))
        .find(|event| event["type"] == "response.output_text.delta")
        .unwrap();
    assert_eq!(delta["delta"], "é💥");
}

#[test]
fn standalone_empty_message_emits_no_delta() {
    let frames = build_block_sse_chunks("", "m", None, false, None, &stamp()).unwrap();
    let events: Vec<Value> = frames.iter().map(|frame| frame_value(frame)).collect();
    let deltas: Vec<&Value> = events
        .iter()
        .filter(|event| event["type"] == "response.output_text.delta")
        .collect();
    assert!(deltas.is_empty());
    let done = events
        .iter()
        .find(|event| event["type"] == "response.output_text.done")
        .unwrap();
    assert_eq!(done["text"], "");
}

#[test]
fn continuation_closes_open_function_call_incomplete_before_block_item() {
    let events = [
        json!({"type":"response.created","response":{"id":"resp_stream","model":"stream-model"}}),
        json!({"type":"response.output_item.added","output_index":4,"item":{"type":"function_call","id":"call_1","call_id":"id","name":"tool","arguments":"{}"}}),
    ];
    let frames =
        build_block_sse_chunks("blocked", "m", None, true, Some(&events), &stamp()).unwrap();
    let close = frame_value(&frames[0]);
    assert_eq!(close["type"], "response.output_item.done");
    assert_eq!(close["item"]["status"], "incomplete");
    assert_eq!(close["item"]["arguments"], "{}");
    let completed = frame_value(frames.last().unwrap());
    assert_eq!(completed["response"]["id"], "resp_stream");
    assert_eq!(completed["response"]["model"], "stream-model");
    assert_eq!(completed["response"]["output"].as_array().unwrap().len(), 1);
    assert_eq!(completed["response"]["output"][0]["id"], "msg_fixed");
}

#[test]
fn continuation_closes_open_message_and_adds_block_item() {
    let events = [
        json!({"type":"response.created","response":{"id":"resp_stream","model":"stream-model"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_open","role":"assistant"}}),
        json!({"type":"response.content_part.added","item_id":"msg_open","content_index":0}),
        json!({"type":"response.output_text.delta","item_id":"msg_open","delta":"part"}),
    ];
    let frames =
        build_block_sse_chunks("blocked", "m", None, true, Some(&events), &stamp()).unwrap();
    let kinds: Vec<String> = frames
        .iter()
        .map(|frame| frame_value(frame)["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        kinds,
        [
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ]
    );
    let item_done = frame_value(&frames[2]);
    assert_eq!(item_done["item"]["status"], "completed");
    assert_eq!(item_done["item"]["content"][0]["text"], "part");
    let added = frame_value(&frames[3]);
    assert_eq!(added["output_index"], 1);
    let done_part = frame_value(&frames[7]);
    assert_eq!(done_part["part"]["annotations"], json!([]));
    assert!(done_part["part"].get("logprobs").is_none());
    assert!(added.get("sequence_number").is_none());
}

#[test]
fn continuation_identity_skips_empty_fields_and_advances_output_index() {
    let events = [
        json!({"response":{"id":"","model":""},"output_index":1}),
        json!({"response":{"id":"resp_stream"},"output_index":7}),
        json!({"response":{"model":"model_stream"},"output_index":3}),
    ];
    assert_eq!(
        continuation_identity("given-model", Some(&events), &stamp()),
        ("resp_stream".to_owned(), "model_stream".to_owned(), 8)
    );
}

#[test]
fn continuation_identity_falls_back_to_stamp_and_given_model() {
    assert_eq!(
        continuation_identity("given-model", None, &stamp()),
        ("resp_fixed".to_owned(), "given-model".to_owned(), 0)
    );
}

#[test]
fn stream_error_has_guardrail_code_and_next_sequence() {
    let events = [json!({"sequence_number":5}), json!({"type":"event"})];
    let errors = build_stream_error_items(403, "blocked", Some(&events));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].error.r#type, "guardrail_error");
    assert_eq!(errors[0].error.code, "403");
    assert_eq!(errors[0].error.message, "blocked");
    assert!(errors[0].error.param.is_none());
    assert_eq!(errors[0].sequence_number, 6);
    assert!(errors[0].extra.is_empty());
}

#[test]
fn continuation_identity_clamps_an_index_above_the_signed_range() {
    let events = [json!({"output_index": 18446744073709551615_u64})];
    assert_eq!(
        continuation_identity("m", Some(&events), &stamp()),
        ("resp_fixed".to_owned(), "m".to_owned(), i64::MAX)
    );
}

#[test]
fn standalone_cost_calculator_reports_no_cost() {
    let object: indexmap::IndexMap<String, betterlite_models::decisions::Json> =
        indexmap::IndexMap::new();
    let cost = NoCost.response_cost(&CostTarget::Object(&object)).unwrap();
    assert!(cost.is_none());
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
    assert_eq!(
        next_stream_sequence_number(Some(&[
            json!({"sequence_number":"text"}),
            json!({"sequence_number":2})
        ])),
        3
    );
}
