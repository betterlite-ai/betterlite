#![cfg(test)]

use super::{ParseArgumentsError, parse_tool_call_arguments};
use crate::arguments::json::Json;
use rstest::rstest;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

#[rstest]
#[case::empty_string_returns_an_empty_object("", json("{}"))]
#[case::whitespace_only_returns_an_empty_object("   ", json("{}"))]
#[case::valid_json_parses_directly("{\"a\":1}", json(r#"{"a": 1}"#))]
#[case::non_object_json_is_returned_as_is("[1,2,3]", json("[1, 2, 3]"))]
#[case::truncated_json_is_repaired("{\"a\":1,\"b\":[1,2", json(r#"{"a": 1, "b": [1, 2]}"#))]
fn tool_arguments_behavior_tests_behavior(#[case] arguments: &str, #[case] expected: Json) {
    let parsed = parse_tool_call_arguments(arguments, None, None).expect("parses");
    assert_eq!(parsed, expected);
}

#[test]
fn unrepairable_arguments_report_the_original_parse_error_and_inputs() {
    let error = parse_tool_call_arguments("{not json", Some("my_tool"), Some("my_context"))
        .expect_err("not valid or repairable JSON");
    let ParseArgumentsError::Malformed {
        tool_name,
        context,
        arguments,
        ..
    } = error;
    assert_eq!(tool_name, Some("my_tool".to_owned()));
    assert_eq!(context, Some("my_context".to_owned()));
    assert_eq!(arguments, "{not json");
}
