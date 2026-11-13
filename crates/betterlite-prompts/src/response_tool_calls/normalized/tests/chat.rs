use super::*;
use crate::response_tool_calls::normalized::chat_completion_tool_calls;
use rstest::rstest;

#[rstest]
#[case::first_choice_only(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"a","function":{"name":"alpha","arguments":"{\"x\":1}"}}]}},{"message":{"tool_calls":[{"id":"b","function":{"name":"beta","arguments":"{}"}}]}}]}"#,
    false,
    r#"[{"id":"a","name":"alpha","arguments":{"x":1}}]"#
)]
#[case::all_choices_in_order(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"a","function":{"name":"alpha","arguments":"{\"x\":1}"}}]}},{"message":{"tool_calls":[{"id":"b","function":{"name":"beta","arguments":"{}"}}]}}]}"#,
    true,
    r#"[{"id":"a","name":"alpha","arguments":{"x":1}},{"id":"b","name":"beta","arguments":{}}]"#
)]
#[case::choice_without_message_is_skipped(
    r#"{"choices":[{"index":0},{"message":{"tool_calls":[{"id":"c","function":{"name":"gamma","arguments":"{}"}}]}}]}"#,
    true,
    r#"[{"id":"c","name":"gamma","arguments":{}}]"#
)]
#[case::tool_call_without_function_is_skipped(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"no_function"},{"id":"d","function":{"name":"delta","arguments":"{}"}}]}}]}"#,
    false,
    r#"[{"id":"d","name":"delta","arguments":{}}]"#
)]
#[case::absent_tool_calls_yield_nothing(
    r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#,
    false,
    "[]"
)]
#[case::absent_arguments_default_to_empty_object(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"e","function":{"name":"epsilon"}}]}}]}"#,
    false,
    r#"[{"id":"e","name":"epsilon","arguments":{}}]"#
)]
#[case::object_arguments_pass_through(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"f","function":{"name":"phi","arguments":{"k":[1,2]}}}]}}]}"#,
    false,
    r#"[{"id":"f","name":"phi","arguments":{"k":[1,2]}}]"#
)]
#[case::non_object_arguments_give_empty_object(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"g","function":{"name":"gamma","arguments":7}}]}}]}"#,
    false,
    r#"[{"id":"g","name":"gamma","arguments":{}}]"#
)]
#[case::empty_id_becomes_null(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"","function":{"name":"eta","arguments":"{}"}}]}}]}"#,
    false,
    r#"[{"id":null,"name":"eta","arguments":{}}]"#
)]
#[case::empty_name_is_preserved(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"h","function":{"name":"","arguments":"{}"}}]}}]}"#,
    false,
    r#"[{"id":"h","name":"","arguments":{}}]"#
)]
#[case::missing_id_becomes_null(
    r#"{"choices":[{"message":{"tool_calls":[{"function":{"name":"iota","arguments":"{}"}}]}}]}"#,
    false,
    r#"[{"id":null,"name":"iota","arguments":{}}]"#
)]
#[case::empty_choices_yield_nothing(r#"{"choices":[]}"#, true, "[]")]
fn response_tool_calls_behavior_tests_behavior(
    #[case] json: &str,
    #[case] include_all_choices: bool,
    #[case] expected: &str,
) {
    assert_eq!(
        chat_completion_tool_calls(&response(json), include_all_choices),
        calls(expected)
    );
}
