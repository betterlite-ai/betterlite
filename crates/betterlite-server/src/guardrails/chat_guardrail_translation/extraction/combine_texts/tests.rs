#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{TextKey, combine_streaming_texts};
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

fn combine(chunks: Vec<JsonValue>) -> Vec<(TextKey, String)> {
    let responses: Vec<_> = chunks.into_iter().map(object).collect();
    combine_streaming_texts(&responses).into_iter().collect()
}

fn entries(expected: &[(TextKey, &str)]) -> Vec<(TextKey, String)> {
    expected
        .iter()
        .map(|&(key, text)| (key, text.to_owned()))
        .collect()
}

#[rstest]
#[case::string_content_joins_per_choice(
    vec![json(r#"{"choices": [{"index": 0, "delta": {"content": "he"}}, {"index": 1, "delta": {"content": "x"}}]}"#), json(r#"{"choices": [{"index": 0, "delta": {"content": "llo"}}]}"#)],
    &[((0, None), "hello"), ((1, None), "x")],
)]
#[case::first_appearance_order(
    vec![json(r#"{"choices": [{"index": 1, "delta": {"content": "a"}}]}"#), json(r#"{"choices": [{"index": 0, "delta": {"content": "b"}}]}"#)],
    &[((1, None), "a"), ((0, None), "b")],
)]
#[case::empty_string_still_makes_the_key(vec![json(r#"{"choices": [{"index": 0, "delta": {"content": ""}}]}"#)], &[((0, None), "")])]
#[case::list_content_joins_per_item_index(
    vec![json(r#"{"choices": [{"index": 0, "message": {"content": [{"text": "a"}, {"type": "image_url"}, {"text": "b"}]}}]}"#), json(r#"{"choices": [{"index": 0, "message": {"content": [{"text": "c"}]}}]}"#)],
    &[((0, Some(0)), "ac"), ((0, Some(2)), "b")],
)]
#[case::list_items_without_text_make_no_key(
    vec![json(r#"{"choices": [{"index": 0, "delta": {"content": [{"text": ""}, {"text": null}, {}, "x"]}}]}"#)],
    &[],
)]
#[case::missing_or_null_index_is_zero(
    vec![json(r#"{"choices": [{"delta": {"content": "a"}}, {"index": null, "delta": {"content": "b"}}]}"#)],
    &[((0, None), "ab")],
)]
#[case::null_and_absent_content_is_skipped(
    vec![json(r#"{"choices": [{"index": 0, "delta": {"content": null}}, {"index": 1, "delta": {"role": "assistant"}}, {"index": 2}]}"#)],
    &[],
)]
#[case::string_and_list_content_of_one_choice_are_separate_keys(
    vec![json(r#"{"choices": [{"index": 0, "delta": {"content": "a"}}]}"#), json(r#"{"choices": [{"index": 0, "delta": {"content": [{"text": "b"}]}}]}"#)],
    &[((0, None), "a"), ((0, Some(0)), "b")],
)]
#[case::no_chunks(vec![], &[])]
fn streamed_text_is_combined_per_choice_and_content_index(
    #[case] chunks: Vec<JsonValue>,
    #[case] expected: &[(TextKey, &str)],
) {
    assert_eq!(combine(chunks), entries(expected));
}
