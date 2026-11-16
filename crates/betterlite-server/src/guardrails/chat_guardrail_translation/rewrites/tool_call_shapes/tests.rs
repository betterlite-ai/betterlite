#![cfg(test)]

use super::function_tool_call_shapes;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::response;

#[test]
fn tool_call_shapes_list_name_and_arguments_across_choices_in_order() {
    let shapes = function_tool_call_shapes(&response(
        r#"{"choices": [
        {"message": {"tool_calls": [
            {"id": "1", "function": {"name": "f", "arguments": "{\"a\":1}"}},
            {"id": "2", "function": {"name": "g", "arguments": "{}"}}]}},
        {"message": {"content": "text"}},
        {"message": {"tool_calls": [{"id": "3", "function": {"name": "h", "arguments": "[]"}}]}}]}"#,
    ));
    let expected = [
        (Some("f".to_owned()), r#"{"a":1}"#.to_owned()),
        (Some("g".to_owned()), "{}".to_owned()),
        (Some("h".to_owned()), "[]".to_owned()),
    ];
    assert_eq!(shapes, expected);
}

#[test]
fn tool_call_shapes_keep_a_missing_name_and_blank_missing_arguments() {
    let shapes = function_tool_call_shapes(&response(
        r#"{"choices": [{"message": {"tool_calls": [{"id": "1", "function": {}}]}}]}"#,
    ));
    assert_eq!(shapes, [(None, String::new())]);
}

#[test]
fn tool_call_shapes_skip_a_call_without_a_function_and_a_response_without_calls() {
    let shapes = function_tool_call_shapes(&response(
        r#"{"choices": [{"message": {"tool_calls": [{"id": "1"}]}}, {"message": {"content": "x"}}]}"#,
    ));
    assert!(shapes.is_empty());
}
