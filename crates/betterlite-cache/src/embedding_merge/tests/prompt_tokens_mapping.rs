use rstest::rstest;

use super::support::extra;
use crate::embedding_merge::details::details_as_mapping;

#[rstest]
#[case::empty("{}", "{}")]
#[case::top_level_null(
    r#"{"audio_tokens": null, "cached_tokens": 3}"#,
    r#"{"cached_tokens": 3}"#
)]
#[case::nested_null(
    r#"{"cache_creation_token_details": {"ephemeral_5m_input_tokens": null, "ephemeral_1h_input_tokens": 2}}"#,
    r#"{"cache_creation_token_details": {"ephemeral_1h_input_tokens": 2}}"#
)]
#[case::deeper_null(
    r#"{"outer": {"inner": {"gone": null, "kept": 1}}}"#,
    r#"{"outer": {"inner": {"kept": 1}}}"#
)]
#[case::object_inside_a_list(
    r#"{"items": [{"gone": null, "kept": 1}, null]}"#,
    r#"{"items": [{"kept": 1}, null]}"#
)]
#[case::falsy_values_stay(
    r#"{"cached_tokens": 0, "note": "", "flag": false, "items": [], "nested": {}}"#,
    r#"{"cached_tokens": 0, "note": "", "flag": false, "items": [], "nested": {}}"#
)]
fn drops_nulls_at_every_level(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(details_as_mapping(&extra(input)), extra(expected));
}

#[test]
fn keeps_the_order_of_the_fields() {
    let mapping = details_as_mapping(&extra(
        r#"{"text_tokens": 1, "dropped": null, "audio_tokens": 2, "cached_tokens": 3}"#,
    ));
    let names: Vec<&str> = mapping.keys().map(String::as_str).collect();
    assert_eq!(names, ["text_tokens", "audio_tokens", "cached_tokens"]);
}
