use super::*;
use crate::response_tool_calls::normalized::responses_api_tool_calls;
use rstest::rstest;

#[rstest]
#[case::function_call_items_only(
    r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"hi"}]},{"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{\"q\":\"x\"}"},{"type":"reasoning","id":"r"}]}"#,
    r#"[{"id":"call_1","name":"lookup","arguments":{"q":"x"}}]"#
)]
#[case::call_id_preferred_over_id(
    r#"{"output":[{"type":"function_call","call_id":"call_1","id":"fc_1","name":"lookup","arguments":"{}"}]}"#,
    r#"[{"id":"call_1","name":"lookup","arguments":{}}]"#
)]
#[case::id_used_when_call_id_is_absent(
    r#"{"output":[{"type":"function_call","id":"fc_1","name":"lookup","arguments":"{}"}]}"#,
    r#"[{"id":"fc_1","name":"lookup","arguments":{}}]"#
)]
#[case::empty_call_id_does_not_fall_back_to_id(
    r#"{"output":[{"type":"function_call","call_id":"","id":"fc_1","name":"lookup","arguments":"{}"}]}"#,
    r#"[{"id":null,"name":"lookup","arguments":{}}]"#
)]
#[case::non_function_items_are_skipped(
    r#"{"output":[{"type":"message","content":"hi"},{"type":"reasoning","id":"r"}]}"#,
    "[]"
)]
#[case::absent_output_yields_nothing("{}", "[]")]
#[case::absent_arguments_default_to_empty_object(
    r#"{"output":[{"type":"function_call","call_id":"c","name":"n"}]}"#,
    r#"[{"id":"c","name":"n","arguments":{}}]"#
)]
#[case::object_arguments_pass_through(
    r#"{"output":[{"type":"function_call","call_id":"c","name":"n","arguments":{"a":1}}]}"#,
    r#"[{"id":"c","name":"n","arguments":{"a":1}}]"#
)]
fn response_tool_calls_behavior_tests_behavior(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(responses_api_tool_calls(&response(json)), calls(expected));
}
