#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_items;
use rstest::rstest;

use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{guardrail, json};

// `_stream_chunk_choices` is stream_item_items(chunk, "choices"); the callers use it directly.
#[rstest]
#[case::list(
    json(r#"{"choices": [{"index": 0}, {"index": 1}]}"#),
    json(r#"[{"index": 0}, {"index": 1}]"#)
)]
#[case::empty_list(json(r#"{"choices": []}"#), json("[]"))]
#[case::missing(json(r#"{"id": "x"}"#), json("[]"))]
#[case::null(json(r#"{"choices": null}"#), json("[]"))]
#[case::string_is_not_a_sequence(json(r#"{"choices": "abc"}"#), json("[]"))]
#[case::object_is_not_a_sequence(json(r#"{"choices": {"index": 0}}"#), json("[]"))]
#[case::chunk_that_is_not_an_object(json(r#"["choices"]"#), json("[]"))]
fn a_chunk_yields_its_choices_only_when_they_are_a_list(
    #[case] chunk: JsonValue,
    #[case] expected: JsonValue,
) {
    assert_eq!(
        serde_json::to_value(stream_item_items(
            &StreamItem::Json(guardrail(chunk)),
            "choices"
        ))
        .unwrap(),
        expected
    );
}

#[test]
fn a_bytes_chunk_has_no_choices() {
    let chunk = StreamItem::Bytes(b"data: {}".to_vec());
    assert!(stream_item_items(&chunk, "choices").is_empty());
}
