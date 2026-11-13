use super::*;
use crate::response_tool_calls::normalized::get_tool_calls_from_response;
use rstest::rstest;

#[rstest]
#[case::chat_wins_over_the_other_sections(
    r#"{
        "choices":[{"message":{"tool_calls":[{"id":"chat","function":{"name":"from_chat","arguments":"{}"}}]}}],
        "output":[{"type":"function_call","call_id":"out","name":"from_output","arguments":"{}"}],
        "content":[{"type":"tool_use","id":"block","name":"from_content","input":{}}]
    }"#,
    r#"[{"id":"chat","name":"from_chat","arguments":{}}]"#
)]
#[case::chat_without_calls_falls_through_to_output(
    r#"{
        "choices":[{"message":{"role":"assistant","content":"no calls here"}}],
        "output":[{"type":"function_call","call_id":"out","name":"from_output","arguments":"{}"}],
        "content":[{"type":"tool_use","id":"block","name":"from_content","input":{}}]
    }"#,
    r#"[{"id":"out","name":"from_output","arguments":{}}]"#
)]
#[case::output_without_calls_falls_through_to_content(
    r#"{
        "choices":[],
        "output":[{"type":"message","content":"no calls here"}],
        "content":[{"type":"tool_use","id":"block","name":"from_content","input":{}}]
    }"#,
    r#"[{"id":"block","name":"from_content","arguments":{}}]"#
)]
#[case::nothing_in_any_section_yields_no_calls("{}", "[]")]
#[case::empty_sections_yield_no_calls(r#"{"choices":[],"output":[],"content":[]}"#, "[]")]
fn response_tool_calls_behavior_tests_behavior(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(
        get_tool_calls_from_response(&response(json), false),
        calls(expected)
    );
}
