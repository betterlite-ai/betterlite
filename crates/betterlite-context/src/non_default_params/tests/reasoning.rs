#![cfg(test)]

use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::params;
use crate::non_default_params::reasoning_aliases::{
    peek_reasoning_summary_aliases, strip_reasoning_aliases,
};

fn text(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

#[rstest]
#[case::camel_on_top(
    r#"{"reasoningSummary": "c", "reasoning_summary": "s"}"#,
    Some(text("c"))
)]
#[case::snake_on_top(r#"{"reasoning_summary": "s"}"#, Some(text("s")))]
#[case::top_beats_nested(
    r#"{"reasoning_summary": "s", "extra_body": {"reasoningSummary": "n"}}"#,
    Some(text("s"))
)]
#[case::nested_camel(
    r#"{"extra_body": {"reasoningSummary": "nc", "reasoning_summary": "ns"}}"#,
    Some(text("nc"))
)]
#[case::nested_snake(r#"{"extra_body": {"reasoning_summary": "ns"}}"#, Some(text("ns")))]
#[case::empty_string_is_a_value(
    r#"{"reasoningSummary": "", "reasoning_summary": "s"}"#,
    Some(text(""))
)]
#[case::null_camel_decides_over_snake(
    r#"{"reasoningSummary": null, "reasoning_summary": "s"}"#,
    None
)]
#[case::null_top_decides_over_nested(
    r#"{"reasoning_summary": null, "extra_body": {"reasoning_summary": "n"}}"#,
    None
)]
#[case::extra_body_that_is_not_an_object(r#"{"extra_body": "x"}"#, None)]
#[case::nothing_set(r#"{"n": 1}"#, None)]
fn peek_alias_order(#[case] input: &str, #[case] expected: Option<JsonValue>) {
    let optional = params(input);
    assert_eq!(peek_reasoning_summary_aliases(&optional), expected.as_ref());
}

#[rstest]
#[case::camel_first(
    r#"{"reasoningSummary": "c", "reasoning_summary": "s", "n": 1}"#,
    r#"{"n": 1}"#,
    Some(text("c"))
)]
#[case::null_camel_falls_to_snake(
    r#"{"reasoningSummary": null, "reasoning_summary": "s"}"#,
    "{}",
    Some(text("s"))
)]
#[case::nested_camel_before_nested_snake(
    r#"{"extra_body": {"reasoningSummary": "nc", "reasoning_summary": "ns", "keep": 1}}"#,
    r#"{"extra_body": {"keep": 1}}"#,
    Some(text("nc"))
)]
#[case::nested_snake_only(
    r#"{"extra_body": {"reasoning_summary": "ns", "keep": 1}}"#,
    r#"{"extra_body": {"keep": 1}}"#,
    Some(text("ns"))
)]
#[case::top_before_nested_and_both_are_removed(
    r#"{"reasoning_summary": "s", "extra_body": {"reasoningSummary": "n", "keep": 1}}"#,
    r#"{"extra_body": {"keep": 1}}"#,
    Some(text("s"))
)]
#[case::emptied_extra_body_is_removed(
    r#"{"extra_body": {"reasoning_summary": "ns"}, "n": 1}"#,
    r#"{"n": 1}"#,
    Some(text("ns"))
)]
#[case::empty_extra_body_is_removed_too(r#"{"extra_body": {}}"#, "{}", None)]
#[case::all_null_values_give_none(
    r#"{"reasoningSummary": null, "reasoning_summary": null, "extra_body": {"reasoningSummary": null, "x": 1}}"#,
    r#"{"extra_body": {"x": 1}}"#,
    None
)]
#[case::extra_body_that_is_not_an_object_stays(
    r#"{"extra_body": "x"}"#,
    r#"{"extra_body": "x"}"#,
    None
)]
fn strip_alias_order(
    #[case] input: &str,
    #[case] expected_params: &str,
    #[case] expected_summary: Option<JsonValue>,
) {
    let original = params(input);
    let (stripped, summary) = strip_reasoning_aliases(&original);
    assert_eq!(stripped, params(expected_params));
    assert_eq!(summary, expected_summary);
    assert_eq!(original, params(input));
}
