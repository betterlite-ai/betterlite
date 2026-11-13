#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::accumulate_by_choice_index;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

fn accumulate(chunks: Vec<JsonValue>) -> Vec<(i64, String)> {
    let responses: Vec<_> = chunks.into_iter().map(object).collect();
    accumulate_by_choice_index(&responses).into_iter().collect()
}

fn pairs(expected: &[(i64, &str)]) -> Vec<(i64, String)> {
    expected
        .iter()
        .map(|&(index, text)| (index, text.to_owned()))
        .collect()
}

#[rstest]
#[case::joins_chunks_of_one_choice(
    vec![json(r#"{"choices": [{"index": 0, "delta": {"content": "he"}}]}"#), json(r#"{"choices": [{"index": 0, "delta": {"content": "llo"}}]}"#)],
    &[(0, "hello")],
)]
#[case::keys_by_the_choice_index_not_the_position(
    vec![json(r#"{"choices": [{"index": 2, "delta": {"content": "a"}}]}"#), json(r#"{"choices": [{"index": 2, "delta": {"content": "b"}}]}"#)],
    &[(2, "ab")],
)]
#[case::first_appearance_order(
    vec![json(r#"{"choices": [{"index": 1, "delta": {"content": "x"}}, {"index": 0, "delta": {"content": "y"}}]}"#), json(r#"{"choices": [{"index": 1, "delta": {"content": "z"}}]}"#)],
    &[(1, "xz"), (0, "y")],
)]
#[case::missing_or_null_index_is_zero(
    vec![json(r#"{"choices": [{"delta": {"content": "a"}}, {"index": null, "delta": {"content": "b"}}]}"#)],
    &[(0, "ab")],
)]
#[case::completed_choices_use_the_message(
    vec![json(r#"{"choices": [{"index": 0, "message": {"content": "done"}}]}"#)],
    &[(0, "done")],
)]
#[case::list_null_and_absent_content_is_skipped(
    vec![json(r#"{"choices": [
        {"index": 0, "delta": {"content": [{"text": "a"}]}},
        {"index": 1, "delta": {"content": null}},
        {"index": 2, "delta": {"role": "assistant"}},
        {"index": 3}
    ]}"#)],
    &[],
)]
#[case::empty_string_adds_no_key(vec![json(r#"{"choices": [{"index": 4, "delta": {"content": ""}}]}"#)], &[])]
#[case::chunk_without_choices(vec![json(r#"{"id": "x"}"#), json(r#"{"choices": null}"#)], &[])]
#[case::no_chunks(vec![], &[])]
fn string_content_is_joined_per_choice_index(
    #[case] chunks: Vec<JsonValue>,
    #[case] expected: &[(i64, &str)],
) {
    assert_eq!(accumulate(chunks), pairs(expected));
}
