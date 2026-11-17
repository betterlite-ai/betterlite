#![cfg(test)]

use rstest::rstest;
use serde_json::{Value, json};

use super::{streaming_has_ended, streaming_scan_key, streaming_string_so_far};

fn delta(item_id: &str, output_index: u32, content_index: u32, value: &str) -> Value {
    json!({
        "type": "response.output_text.delta",
        "item_id": item_id,
        "output_index": output_index,
        "content_index": content_index,
        "delta": value,
    })
}

#[test]
fn string_so_far_uses_the_last_done_text_and_ignores_the_part_deltas() {
    let events = [
        delta("m1", 0, 0, "abc"),
        delta("m1", 0, 0, "def"),
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "Hello"}),
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "Hello again"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "Hello again");
}

#[test]
fn string_so_far_joins_the_deltas_of_a_part_without_done_text() {
    let events = [
        delta("m1", 0, 0, "ab"),
        delta("m1", 0, 1, "cd"),
        delta("m1", 0, 0, "e"),
    ];
    assert_eq!(streaming_string_so_far(&events), "abecd");
}

#[test]
fn string_so_far_orders_parts_by_first_appearance() {
    let events = [
        json!({"type": "response.output_text.done", "item_id": "m2", "output_index": 1, "content_index": 0, "text": "second"}),
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "first"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "secondfirst");
}

#[test]
fn string_so_far_counts_any_event_carrying_a_text_or_delta_string() {
    let events = [
        json!({"type": "response.reasoning_summary_text.delta", "item_id": "r1", "output_index": 0, "summary_index": 0, "delta": "why"}),
        json!({"type": "response.function_call_arguments.delta", "item_id": "f1", "output_index": 1, "delta": "{}"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "why{}");
}

#[test]
fn string_so_far_skips_text_and_delta_values_that_are_not_strings() {
    let events = [
        json!({"type": "response.output_text.delta", "item_id": "m1", "output_index": 0, "content_index": 0, "text": 5}),
        json!({"type": "response.output_text.delta", "item_id": "m2", "output_index": 1, "content_index": 0, "delta": null}),
        json!({"type": "response.output_text.delta", "item_id": "m3", "output_index": 2, "content_index": 0, "text": ["x"]}),
        json!("bare string"),
        json!({"type": "response.output_text.done", "item_id": "m4", "output_index": 3, "content_index": 0, "text": "kept"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "kept");
}

#[test]
fn string_so_far_reads_a_missing_index_and_a_null_index_as_the_same_part() {
    let events = [
        json!({"type": "response.output_text.done", "text": "a"}),
        json!({"type": "response.output_text.delta", "item_id": null, "output_index": null, "content_index": null, "delta": "b"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "a");
}

#[test]
fn string_so_far_keeps_two_item_ids_at_one_position_apart() {
    let events = [
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "a"}),
        json!({"type": "response.output_text.done", "item_id": "m2", "output_index": 0, "content_index": 0, "text": "b"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "ab");
}

#[test]
fn string_so_far_keeps_one_item_id_at_two_output_indexes_apart() {
    let events = [
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "a"}),
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 1, "content_index": 0, "text": "b"}),
    ];
    assert_eq!(streaming_string_so_far(&events), "ab");
}

#[test]
fn string_so_far_joins_nothing_for_an_empty_stream() {
    assert_eq!(streaming_string_so_far(&[]), "");
}

#[rstest]
#[case::completed("response.completed")]
#[case::failed("response.failed")]
#[case::incomplete("response.incomplete")]
fn ended_is_true_for_a_terminal_envelope_type(#[case] event_type: &str) {
    assert!(streaming_has_ended(&[json!({"type": event_type})]));
}

#[test]
fn ended_is_false_for_an_empty_stream() {
    assert!(!streaming_has_ended(&[]));
}

#[test]
fn ended_is_false_when_the_last_event_is_not_terminal() {
    let events = [
        json!({"type": "response.completed"}),
        json!({"type": "response.output_text.delta", "delta": "tail"}),
    ];
    assert!(!streaming_has_ended(&events));
}

#[test]
fn ended_is_false_when_the_last_event_is_not_an_object() {
    assert!(!streaming_has_ended(&[json!("bare string")]));
}

#[test]
fn ended_is_false_when_the_last_event_has_no_type() {
    assert!(!streaming_has_ended(&[json!({"response": {}})]));
}

#[test]
fn scan_key_is_none_for_an_empty_stream() {
    assert!(streaming_scan_key(&[]).is_none());
}

#[rstest]
#[case::string(json!("bare string"))]
#[case::list(json!([{"type": "response.output_text.delta"}]))]
fn scan_key_is_none_when_the_last_event_is_not_an_object(#[case] event: Value) {
    assert!(streaming_scan_key(&[event]).is_none());
}

#[rstest]
#[case::without_index(json!({"type": "response.output_item.done", "item": {"type": "function_call"}}))]
#[case::with_index(json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "message"}}))]
fn scan_key_is_none_for_an_output_item_done_last_event(#[case] event: Value) {
    assert!(streaming_scan_key(&[event]).is_none());
}

#[test]
fn scan_key_completed_envelope_collects_the_non_empty_message_texts() {
    let events = [json!({
        "type": "response.completed",
        "response": {"output": [
            {"type": "message", "content": [
                {"type": "output_text", "text": "hello"},
                {"type": "output_text", "text": ""},
                {"type": "refusal", "refusal": "no"},
            ]},
            {"type": "message", "content": [{"type": "output_text", "text": " world"}]},
            {"type": "reasoning", "summary": []},
            "bare",
        ]},
    })];
    let key = streaming_scan_key(&events).unwrap();
    assert_eq!(key.texts, ["hello", " world"]);
    assert!(key.tool_calls.is_empty());
    assert!(key.stream_ended);
    assert!(!key.tool_calls_in_flight);
}

#[test]
fn scan_key_completed_envelope_fingerprints_each_tool_call_item() {
    let events = [json!({
        "type": "response.completed",
        "response": {"output": [
            {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "lookup", "arguments": "{\"q\":1}"},
            {"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_2", "name": "grammar", "input": "raw"},
            {"type": "message", "content": [{"type": "output_text", "text": "hi"}]},
        ]},
    })];
    let key = streaming_scan_key(&events).unwrap();
    assert_eq!(key.texts, ["hi"]);
    assert_eq!(
        key.tool_calls,
        [
            r#"{"arguments":"{\"q\":1}","call_id":"call_1","id":"fc_1","name":"lookup","type":"function_call"}"#,
            r#"{"call_id":"call_2","id":"ctc_1","input":"raw","name":"grammar","type":"custom_tool_call"}"#,
        ]
    );
    assert!(key.stream_ended);
    assert!(!key.tool_calls_in_flight);
}

#[rstest]
#[case::missing(json!({"type": "response.failed"}))]
#[case::null(json!({"type": "response.incomplete", "response": null}))]
#[case::not_an_object(json!({"type": "response.completed", "response": "partial"}))]
fn scan_key_completed_envelope_without_output_has_no_contents(#[case] event: Value) {
    let key = streaming_scan_key(&[event]).unwrap();
    assert!(key.texts.is_empty());
    assert!(key.tool_calls.is_empty());
    assert!(key.stream_ended);
    assert!(!key.tool_calls_in_flight);
}

#[test]
fn scan_key_running_stream_carries_the_string_so_far_and_no_tool_calls() {
    let events = [
        delta("m1", 0, 0, "abc"),
        delta("m1", 0, 0, "def"),
        json!({"type": "response.output_text.done", "item_id": "m1", "output_index": 0, "content_index": 0, "text": "hello"}),
    ];
    let key = streaming_scan_key(&events).unwrap();
    assert_eq!(key.texts, ["hello"]);
    assert!(key.tool_calls.is_empty());
    assert!(!key.stream_ended);
    assert!(!key.tool_calls_in_flight);
}

#[test]
fn scan_key_running_stream_keeps_one_empty_text_when_nothing_was_said() {
    let events = [
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "message"}}),
    ];
    let key = streaming_scan_key(&events).unwrap();
    assert_eq!(key.texts, [""]);
    assert!(!key.stream_ended);
    assert!(!key.tool_calls_in_flight);
}

#[test]
fn scan_key_running_stream_flags_a_tool_call_payload_event() {
    let events = [
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "output_index": 1, "delta": "{}"}),
        delta("m1", 0, 0, "hello"),
    ];
    let key = streaming_scan_key(&events).unwrap();
    assert!(key.tool_calls_in_flight);
}

#[test]
fn scan_key_running_stream_flags_an_output_item_event_for_a_tool_call() {
    let events = [
        json!({"type": "response.output_item.added", "output_index": 1, "item": {"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_1", "input": ""}}),
        delta("m1", 0, 0, "hello"),
    ];
    let key = streaming_scan_key(&events).unwrap();
    assert!(key.tool_calls_in_flight);
}

#[test]
fn scan_key_running_stream_leaves_a_message_output_item_unflagged() {
    let events = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "message", "content": []}}),
        delta("m1", 0, 0, "hello"),
    ];
    let key = streaming_scan_key(&events).unwrap();
    assert!(!key.tool_calls_in_flight);
}
