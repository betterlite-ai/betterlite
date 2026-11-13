#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::tool_call_fingerprints;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, stream_items};

fn fingerprints(chunks: Vec<JsonValue>) -> Vec<String> {
    let items = stream_items(chunks);
    tool_call_fingerprints(&items).unwrap()
}

#[rstest]
#[case::no_chunks(vec![], &[])]
#[case::chunk_without_choices(vec![json(r#"{"id": "x"}"#)], &[])]
#[case::choice_without_a_delta(vec![json(r#"{"choices": [{"index": 0}]}"#)], &[])]
#[case::delta_without_tool_calls(vec![json(r#"{"choices": [{"delta": {"content": "hi"}}]}"#)], &[])]
#[case::one_tool_call(
    vec![json(r#"{"choices": [{"delta": {"tool_calls": [{"id": "1"}]}}]}"#)],
    &[r#"{"id":"1"}"#],
)]
#[case::every_choice_of_every_chunk_in_order(
    vec![
        json(r#"{"choices": [{"delta": {"tool_calls": [{"id": "1"}]}}, {"delta": {"tool_calls": [{"id": "2"}, {"id": "3"}]}}]}"#),
        json(r#"{"id": "skip"}"#),
        json(r#"{"choices": [{"delta": {"tool_calls": [{"id": "4"}]}}]}"#),
    ],
    &[r#"{"id":"1"}"#, r#"{"id":"2"}"#, r#"{"id":"3"}"#, r#"{"id":"4"}"#],
)]
#[case::legacy_function_call_follows_the_tool_calls(
    vec![json(r#"{"choices": [{"delta": {"function_call": {"name": "f"}, "tool_calls": [{"id": "1"}]}}]}"#)],
    &[r#"{"id":"1"}"#, r#"{"name":"f"}"#],
)]
#[case::key_order_does_not_change_the_fingerprint(
    vec![
        json(r#"{"choices": [{"delta": {"tool_calls": [{"id": "1", "type": "function"}]}}]}"#),
        json(r#"{"choices": [{"delta": {"tool_calls": [{"type": "function", "id": "1"}]}}]}"#),
    ],
    &[r#"{"id":"1","type":"function"}"#, r#"{"id":"1","type":"function"}"#],
)]
fn each_streamed_tool_call_gets_a_fingerprint(
    #[case] chunks: Vec<JsonValue>,
    #[case] expected: &[&str],
) {
    assert_eq!(fingerprints(chunks), expected);
}
