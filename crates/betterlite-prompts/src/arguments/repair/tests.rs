#![cfg(test)]

use super::attempt_json_repair;
use rstest::rstest;
use serde_json::json;

#[rstest]
#[case::balanced_object_is_not_repaired("{\"a\":1}", None)]
#[case::whitespace_only_is_not_repaired("   ", None)]
#[case::unmatched_object_closes(
    "{\"a\":1,\"b\":[1,2",
    Some(json!({"a": 1, "b": [1, 2]}))
)]
#[case::trailing_comma_is_stripped_before_closing(
    "{\"a\":1,",
    Some(json!({"a": 1}))
)]
#[case::quoted_brace_does_not_push_a_second_opener(
    "{\"a\":\"{\",\"b\":1",
    Some(json!({"a": "{", "b": 1}))
)]
#[case::quoted_brace_inside_truncated_object(
    "{\"a\":\"}{[\",\"b\":2",
    Some(json!({"a": "}{[", "b": 2}))
)]
#[case::escaped_backslash_then_quoted_brace_stays_inert(
    "{\"a\":\"\\\\\",\"b\":\"}\"",
    Some(json!({"a": "\\", "b": "}"}))
)]
#[case::still_unparseable_after_closing_stays_none("{\"a\":,", None)]
fn tool_arguments_behavior_tests_behavior(
    #[case] text: &str,
    #[case] expected: Option<serde_json::Value>,
) {
    assert_eq!(attempt_json_repair(text), expected);
}
