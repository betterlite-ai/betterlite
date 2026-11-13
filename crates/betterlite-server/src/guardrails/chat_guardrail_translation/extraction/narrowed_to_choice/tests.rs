#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::narrowed_to_choice;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

#[rstest]
#[case::keeps_only_the_matching_choice(
    json(r#"{"choices": [{"index": 0, "delta": {"content": "a"}}, {"index": 1, "delta": {"content": "b"}}]}"#),
    1,
    json(r#"{"choices": [{"index": 1, "delta": {"content": "b"}}]}"#),
)]
#[case::keeps_every_choice_with_that_index(
    json(r#"{"choices": [{"index": 1, "n": 1}, {"index": 0}, {"index": 1, "n": 2}]}"#),
    1,
    json(r#"{"choices": [{"index": 1, "n": 1}, {"index": 1, "n": 2}]}"#)
)]
#[case::other_fields_are_copied(
    json(r#"{"id": "c1", "model": "m", "usage": {"total_tokens": 3}, "choices": [{"index": 0}]}"#),
    0,
    json(r#"{"id": "c1", "model": "m", "usage": {"total_tokens": 3}, "choices": [{"index": 0}]}"#)
)]
#[case::a_choice_without_an_index_is_choice_zero(
    json(r#"{"choices": [{"delta": {}}, {"index": null}, {"index": 1}]}"#),
    0,
    json(r#"{"choices": [{"delta": {}}, {"index": null}]}"#)
)]
#[case::no_choice_matches(json(r#"{"choices": [{"index": 0}]}"#), 2, json(r#"{"choices": []}"#))]
#[case::response_without_choices(
    json(r#"{"id": "c1"}"#),
    0,
    json(r#"{"id": "c1", "choices": []}"#)
)]
fn the_response_keeps_only_the_choices_with_that_index(
    #[case] response: JsonValue,
    #[case] index: i64,
    #[case] expected: JsonValue,
) {
    assert_eq!(
        serde_json::to_value(narrowed_to_choice(&object(response), index)).unwrap(),
        expected
    );
}

#[test]
fn the_original_response_is_not_changed() {
    let response = object(json(r#"{"choices": [{"index": 0}, {"index": 1}]}"#));
    let _narrowed = narrowed_to_choice(&response, 1);
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json(r#"{"choices": [{"index": 0}, {"index": 1}]}"#)
    );
}
