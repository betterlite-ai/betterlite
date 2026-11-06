use super::*;
use crate::response_tool_calls::normalized::chat_completion_tool_calls;
use rstest::rstest;

fn chat_with_arguments(arguments: &str) -> ToolCallResponse {
    let escaped = arguments.replace('\\', "\\\\").replace('"', "\\\"");
    response(&format!(
        r#"{{"choices":[{{"message":{{"tool_calls":[{{"id":"call","function":{{"name":"tool","arguments":"{escaped}"}}}}]}}}}]}}"#
    ))
}

#[rstest]
#[case::truncated_json_is_repaired(
    r#"{"a":1,"b":[1,2"#,
    r#"[{"id":"call","name":"tool","arguments":{"a":1,"b":[1,2]}}]"#
)]
#[case::unrepairable_json_gives_empty_arguments(
    "{not json",
    r#"[{"id":"call","name":"tool","arguments":{}}]"#
)]
#[case::redacted_arguments_give_empty_arguments(
    "redacted-by-litellm",
    r#"[{"id":"call","name":"tool","arguments":{}}]"#
)]
#[case::empty_arguments_give_empty_object("", r#"[{"id":"call","name":"tool","arguments":{}}]"#)]
#[case::blank_arguments_give_empty_object("   ", r#"[{"id":"call","name":"tool","arguments":{}}]"#)]
#[case::concatenated_objects_become_separate_calls(
    r#"{"a":1}{"a":2}"#,
    r#"[{"id":"call","name":"tool","arguments":{"a":1}},{"id":"call__concat_1","name":"tool","arguments":{"a":2}}]"#
)]
#[case::identical_concatenated_objects_collapse(
    r#"{"a":1}{"a":1}{"a":1}"#,
    r#"[{"id":"call","name":"tool","arguments":{"a":1}}]"#
)]
#[case::eight_distinct_objects_are_kept(
    r#"{"n":1}{"n":2}{"n":3}{"n":4}{"n":5}{"n":6}{"n":7}{"n":8}"#,
    r#"[
        {"id":"call","name":"tool","arguments":{"n":1}},
        {"id":"call__concat_1","name":"tool","arguments":{"n":2}},
        {"id":"call__concat_2","name":"tool","arguments":{"n":3}},
        {"id":"call__concat_3","name":"tool","arguments":{"n":4}},
        {"id":"call__concat_4","name":"tool","arguments":{"n":5}},
        {"id":"call__concat_5","name":"tool","arguments":{"n":6}},
        {"id":"call__concat_6","name":"tool","arguments":{"n":7}},
        {"id":"call__concat_7","name":"tool","arguments":{"n":8}}
    ]"#
)]
#[case::more_than_eight_distinct_objects_are_dropped(
    r#"{"n":1}{"n":2}{"n":3}{"n":4}{"n":5}{"n":6}{"n":7}{"n":8}{"n":9}"#,
    r#"[{"id":"call","name":"tool","arguments":{}}]"#
)]
#[case::non_object_json_gives_empty_arguments(
    "[1,2]",
    r#"[{"id":"call","name":"tool","arguments":{}}]"#
)]
fn response_tool_calls_behavior_tests_behavior(#[case] arguments: &str, #[case] expected: &str) {
    assert_eq!(
        chat_completion_tool_calls(&chat_with_arguments(arguments), false),
        calls(expected)
    );
}
