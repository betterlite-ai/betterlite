#![cfg(test)]

use rstest::rstest;

use super::coerce_bool;
use crate::sections::settings::settings_rules::JsonValue;

fn parse(json: &str) -> JsonValue {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::lowercase_true(r#""true""#, "true")]
#[case::uppercase_true(r#""TRUE""#, "true")]
#[case::mixed_case_true(r#""True""#, "true")]
#[case::lowercase_false(r#""false""#, "false")]
#[case::mixed_case_false(r#""False""#, "false")]
#[case::other_text_is_false(r#""yes""#, "false")]
#[case::padded_true_is_false(r#"" true""#, "false")]
#[case::empty_text_is_false(r#""""#, "false")]
#[case::null_stays_null("null", "null")]
#[case::true_stays_true("true", "true")]
#[case::false_stays_false("false", "false")]
#[case::zero_is_false("0", "false")]
#[case::float_zero_is_false("0.0", "false")]
#[case::negative_float_zero_is_false("-0.0", "false")]
#[case::one_is_true("1", "true")]
#[case::negative_is_true("-3", "true")]
#[case::fraction_is_true("0.5", "true")]
#[case::empty_list_is_false("[]", "false")]
#[case::list_is_true("[0]", "true")]
#[case::empty_object_is_false("{}", "false")]
#[case::object_is_true(r#"{"a": false}"#, "true")]
fn normalizes_reload_values(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(coerce_bool(parse(input)), parse(expected));
}
