#![cfg(test)]

use super::estimate_json_bytes;
use crate::arguments::json::Json;
use rstest::rstest;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

#[rstest]
#[case::null(json("null"), 4)]
#[case::bool_true(json("true"), 4)]
#[case::bool_false(json("false"), 5)]
#[case::number(json("42"), 24)]
#[case::short_string(json("\"ab\""), 4)]
#[case::empty_list(json("[]"), 2)]
#[case::two_null_items(json("[null, null]"), 11)]
#[case::empty_object(json("{}"), 2)]
#[case::one_null_field(json("{\"a\": null}"), 11)]
#[case::two_fields(json("{\"ab\": 1, \"xyz\": true}"), 43)]
fn schema_definitions_behavior_tests_behavior(#[case] value: Json, #[case] expected: usize) {
    assert_eq!(estimate_json_bytes(&value), expected);
}
