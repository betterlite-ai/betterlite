use betterlite_models::decisions::{Extra, Json};
use rstest::rstest;
use serde_json::Number;

use super::support::{extra, largest_count};
use crate::embedding_merge::details::merge_prompt_tokens_details;
use crate::embedding_merge::error::MergeError;

fn merged(first: Option<&str>, second: Option<&str>) -> Option<Extra> {
    let left = first.map(extra);
    let right = second.map(extra);
    merge_prompt_tokens_details(left.as_ref(), right.as_ref()).expect("the details merge")
}

#[rstest]
#[case::both_absent(None, None, None)]
#[case::only_first_is_returned_as_given(
    Some(r#"{"cached_tokens": 4, "audio_tokens": null}"#),
    None,
    Some(r#"{"cached_tokens": 4, "audio_tokens": null}"#)
)]
#[case::only_second_is_returned_as_given(
    None,
    Some(r#"{"cached_tokens": 5, "audio_tokens": null}"#),
    Some(r#"{"cached_tokens": 5, "audio_tokens": null}"#)
)]
#[case::numbers_add_and_missing_fields_count_as_zero(
    Some(r#"{"cached_tokens": 2, "text_tokens": 1}"#),
    Some(r#"{"cached_tokens": 3, "audio_tokens": 4}"#),
    Some(r#"{"cached_tokens": 5, "text_tokens": 1, "audio_tokens": 4}"#)
)]
#[case::unknown_numbers_add(
    Some(r#"{"custom_tokens": 3}"#),
    Some(r#"{"custom_tokens": 4}"#),
    Some(r#"{"custom_tokens": 7}"#)
)]
#[case::zero_is_kept(
    Some(r#"{"cached_tokens": 0}"#),
    Some("{}"),
    Some(r#"{"cached_tokens": 0}"#)
)]
#[case::nulls_are_dropped(
    Some(r#"{"cached_tokens": null, "text_tokens": 1}"#),
    Some(r#"{"cached_tokens": 2, "audio_tokens": null}"#),
    Some(r#"{"cached_tokens": 2, "text_tokens": 1}"#)
)]
#[case::both_empty(Some("{}"), Some("{}"), None)]
#[case::only_nulls(Some(r#"{"a": null}"#), Some(r#"{"b": null}"#), None)]
fn merges_the_top_level_fields(
    #[case] first: Option<&str>,
    #[case] second: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(merged(first, second), expected.map(extra));
}

#[rstest]
#[case::nested_numbers_add(
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 1}}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 2, "ephemeral_1h_input_tokens": 3}}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 3, "ephemeral_1h_input_tokens": 3}}"#
)]
#[case::cached_tokens_details_add(
    r#"{"cached_tokens_details": {"text_tokens": 1, "audio_tokens": 2}}"#,
    r#"{"cached_tokens_details": {"text_tokens": 4, "image_tokens": 8}}"#,
    r#"{"cached_tokens_details": {"text_tokens": 5, "audio_tokens": 2, "image_tokens": 8}}"#
)]
#[case::nested_only_in_the_first(
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 2}}"#,
    r#"{"cached_tokens": 1}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 2}, "cached_tokens": 1}"#
)]
#[case::nested_only_in_the_second(
    r#"{"cached_tokens": 1}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 2}}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 2}, "cached_tokens": 1}"#
)]
#[case::nested_members_outside_the_model_are_dropped(
    r#"{"cache_creation_token_details": {"other": 1, "ephemeral_5m_input_tokens": 2}}"#,
    r#"{"cache_creation_token_details": {"other": 4}}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 2}}"#
)]
fn merges_nested_objects_one_level_down(
    #[case] first: &str,
    #[case] second: &str,
    #[case] expected: &str,
) {
    assert_eq!(merged(Some(first), Some(second)), Some(extra(expected)));
}

#[rstest]
#[case::first_text_wins(r#"{"note": "a"}"#, r#"{"note": "b"}"#, r#"{"note": "a"}"#)]
#[case::empty_text_gives_way(r#"{"note": ""}"#, r#"{"note": "b"}"#, r#"{"note": "b"}"#)]
#[case::zero_gives_way_to_text(r#"{"note": 0}"#, r#"{"note": "x"}"#, r#"{"note": "x"}"#)]
#[case::text_beats_a_missing_field(r#"{"note": "a"}"#, "{}", r#"{"note": "a"}"#)]
#[case::a_missing_field_gives_way_to_text("{}", r#"{"note": "b"}"#, r#"{"note": "b"}"#)]
#[case::false_gives_way_to_true(r#"{"flag": false}"#, r#"{"flag": true}"#, r#"{"flag": true}"#)]
#[case::true_stays_over_false(r#"{"flag": true}"#, r#"{"flag": false}"#, r#"{"flag": true}"#)]
#[case::empty_list_gives_way(r#"{"tags": []}"#, r#"{"tags": [1]}"#, r#"{"tags": [1]}"#)]
#[case::list_stays_over_another(r#"{"tags": [2]}"#, r#"{"tags": [1]}"#, r#"{"tags": [2]}"#)]
#[case::empty_object_gives_way(r#"{"meta": {}}"#, r#"{"meta": {"a": 1}}"#, r#"{"meta": {"a": 1}}"#)]
fn keeps_the_first_value_that_is_set(
    #[case] first: &str,
    #[case] second: &str,
    #[case] expected: &str,
) {
    assert_eq!(merged(Some(first), Some(second)), Some(extra(expected)));
}

#[rstest]
#[case::floats_add(
    r#"{"video_length_seconds": 1.5}"#,
    r#"{"video_length_seconds": 2.25}"#,
    r#"{"video_length_seconds": 3.75}"#
)]
#[case::integer_and_float_add(
    r#"{"audio_length_seconds": 1}"#,
    r#"{"audio_length_seconds": 1.5}"#,
    r#"{"audio_length_seconds": 2.5}"#
)]
#[case::seconds_become_floats(
    r#"{"video_length_seconds": 1}"#,
    r#"{"video_length_seconds": 2}"#,
    r#"{"video_length_seconds": 3.0}"#
)]
#[case::a_whole_float_count_becomes_an_integer(
    r#"{"cached_tokens": 1.5}"#,
    r#"{"cached_tokens": 0.5}"#,
    r#"{"cached_tokens": 2}"#
)]
fn adds_integers_and_floats(#[case] first: &str, #[case] second: &str, #[case] expected: &str) {
    assert_eq!(merged(Some(first), Some(second)), Some(extra(expected)));
}

#[rstest]
#[case::fractional_count(r#"{"cached_tokens": 1.5}"#, r#"{"cached_tokens": 1}"#)]
#[case::text_count(r#"{"cached_tokens": "x"}"#, r#"{"text_tokens": 1}"#)]
#[case::text_seconds(r#"{"video_length_seconds": "x"}"#, "{}")]
#[case::nested_not_an_object(r#"{"cached_tokens_details": 5}"#, "{}")]
#[case::nested_text_member(
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": "x"}}"#,
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": "x"}}"#
)]
#[case::nested_fractional_member(
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 1.5}}"#,
    "{}"
)]
fn returns_none_when_the_wrapper_rejects_a_declared_field(
    #[case] first: &str,
    #[case] second: &str,
) {
    assert_eq!(merged(Some(first), Some(second)), None);
}

#[rstest]
#[case::write_total(
    r#"{"cache_write_tokens": 2}"#,
    r#"{"cache_write_tokens": 3}"#,
    r#"{"cache_write_tokens": 5, "cache_creation_tokens": 5}"#
)]
#[case::creation_total(
    r#"{"cache_creation_tokens": 2}"#,
    "{}",
    r#"{"cache_creation_tokens": 2, "cache_write_tokens": 2}"#
)]
#[case::write_wins_over_creation(
    r#"{"cache_write_tokens": 1, "cache_creation_tokens": 9}"#,
    "{}",
    r#"{"cache_write_tokens": 1, "cache_creation_tokens": 1}"#
)]
#[case::integer_creation_input_tokens(
    r#"{"cache_creation_input_tokens": 4}"#,
    r#"{"cache_creation_input_tokens": 1}"#,
    r#"{"cache_creation_input_tokens": 5, "cache_write_tokens": 5, "cache_creation_tokens": 5}"#
)]
#[case::fractional_creation_input_tokens_are_not_mirrored(
    r#"{"cache_creation_input_tokens": 1.5}"#,
    r#"{"cache_creation_input_tokens": 1}"#,
    r#"{"cache_creation_input_tokens": 2.5}"#
)]
#[case::text_creation_input_tokens_are_not_mirrored(
    r#"{"cache_creation_input_tokens": "x"}"#,
    "{}",
    r#"{"cache_creation_input_tokens": "x"}"#
)]
fn mirrors_the_cache_write_and_creation_counts(
    #[case] first: &str,
    #[case] second: &str,
    #[case] expected: &str,
) {
    assert_eq!(merged(Some(first), Some(second)), Some(extra(expected)));
}

#[test]
fn lists_the_first_fields_before_the_fields_only_the_second_has() {
    let result = merged(
        Some(r#"{"text_tokens": 1, "cached_tokens": 1}"#),
        Some(r#"{"audio_tokens": 1, "text_tokens": 1}"#),
    )
    .expect("the details merge");
    let names: Vec<&str> = result.keys().map(String::as_str).collect();
    assert_eq!(names, ["text_tokens", "cached_tokens", "audio_tokens"]);
}

#[test]
fn reports_an_overflow_of_a_top_level_total() {
    let error = merge_prompt_tokens_details(
        Some(&largest_count()),
        Some(&extra(r#"{"cached_tokens": 1}"#)),
    )
    .expect_err("the total does not fit");
    assert!(matches!(
        error,
        MergeError::Overflow {
            field: "prompt_tokens_details"
        }
    ));
}

#[test]
fn reports_an_overflow_of_a_nested_total() {
    let count = Number::from_i128(i128::MAX).expect("the largest count is a number");
    let inner = Extra::from([("ephemeral_5m_input_tokens".to_owned(), Json::Number(count))]);
    let first = Extra::from([(
        "cache_creation_token_details".to_owned(),
        Json::Object(inner),
    )]);
    let second = extra(r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": 1}}"#);
    let error = merge_prompt_tokens_details(Some(&first), Some(&second))
        .expect_err("the nested total does not fit");
    assert!(matches!(error, MergeError::Overflow { .. }));
}

#[test]
fn adds_totals_that_just_fit() {
    let almost = Number::from_i128(i128::MAX - 1).expect("the count is a number");
    let first = Extra::from([("cached_tokens".to_owned(), Json::Number(almost))]);
    let result = merge_prompt_tokens_details(Some(&first), Some(&extra(r#"{"cached_tokens": 1}"#)))
        .expect("the total fits");
    assert_eq!(result, Some(largest_count()));
}
