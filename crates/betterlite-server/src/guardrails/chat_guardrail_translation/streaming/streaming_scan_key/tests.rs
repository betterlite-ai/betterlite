#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_translation::StreamingScanKey;

use super::get_streaming_scan_key;
use crate::guardrails::chat_guardrail_translation::streaming::tests::support::{
    TWO_CHOICES, stream_items,
};

const TOOL_CALL_FINGERPRINT: &str =
    r#"{"function":{"arguments":"","name":"f"},"id":"a","index":0}"#;

const IN_FLIGHT: &str = r#"[{"choices": [{"index": 0, "delta": {"content": "a", "tool_calls": [{"index": 0, "id": "a", "function": {"name": "f", "arguments": ""}}]}}]}]"#;

const FINISHED: &str = r#"[{"choices": [{"index": 0, "delta": {"content": "a", "tool_calls": [{"index": 0, "id": "a", "function": {"name": "f", "arguments": ""}}]}}]},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}]"#;

fn key(source: &str) -> StreamingScanKey {
    get_streaming_scan_key(&stream_items(source)).unwrap()
}

#[test]
fn streaming_scan_key_carries_the_text_of_each_choice_in_order() {
    let scanned = key(TWO_CHOICES);
    assert_eq!(scanned.texts, ["Hello", "Hi"]);
    assert!(scanned.tool_calls.is_empty());
    assert!(!scanned.stream_ended);
    assert!(!scanned.tool_calls_in_flight);
}

#[test]
fn streaming_scan_key_keeps_list_content_per_item() {
    let scanned = key(
        r#"[{"choices": [{"index": 0, "delta": {"content": [{"text": "a"}, {"text": "b"}]}}]}]"#,
    );
    assert_eq!(scanned.texts, ["a", "b"]);
}

#[test]
fn streaming_scan_key_flags_tool_calls_in_flight_and_does_not_scan_them_yet() {
    let scanned = key(IN_FLIGHT);
    assert!(scanned.tool_calls.is_empty());
    assert!(scanned.tool_calls_in_flight);
    assert!(!scanned.stream_ended);
}

#[test]
fn streaming_scan_key_fingerprints_the_tool_calls_once_the_stream_ended() {
    let scanned = key(FINISHED);
    assert_eq!(scanned.tool_calls, [TOOL_CALL_FINGERPRINT]);
    assert!(scanned.stream_ended);
    assert!(!scanned.tool_calls_in_flight);
}

#[test]
fn streaming_scan_key_has_no_tool_calls_for_an_ended_stream_without_any() {
    let scanned =
        key(r#"[{"choices": [{"index": 0, "delta": {"content": "a"}, "finish_reason": "stop"}]}]"#);
    assert!(scanned.tool_calls.is_empty());
    assert!(scanned.stream_ended);
    assert!(!scanned.tool_calls_in_flight);
}

#[test]
fn streaming_scan_key_ignores_a_later_choice_finishing() {
    let scanned = key(
        r#"[{"choices": [{"index": 0, "delta": {"content": "a"}}, {"index": 1, "delta": {}, "finish_reason": "stop"}]}]"#,
    );
    assert!(!scanned.stream_ended);
}

#[test]
fn streaming_scan_key_reads_only_assembled_chunks_for_text() {
    let mut items = stream_items(TWO_CHOICES);
    items.push(StreamItem::Bytes(b"data: {}\n\n".to_vec()));
    items.push(StreamItem::Json(GuardrailValue::Text("raw".to_owned())));
    let scanned = get_streaming_scan_key(&items).unwrap();
    assert_eq!(scanned.texts, ["Hello", "Hi"]);
    assert!(!scanned.stream_ended);
}

#[test]
fn streaming_scan_key_of_no_chunks_has_nothing_to_scan() {
    let scanned = get_streaming_scan_key(&[]).unwrap();
    assert!(scanned.has_nothing_to_scan());
}
