use super::*;
use crate::response_tool_calls::normalized::chat_completion_tool_calls;
use rstest::rstest;

#[rstest]
#[case::suffix_skips_ids_another_call_owns(
    r#"{"choices":[{"message":{"tool_calls":[
        {"id":"call","function":{"name":"tool","arguments":"{\"a\":1}{\"a\":2}"}},
        {"id":"call__concat_1","function":{"name":"tool","arguments":"{}"}}
    ]}}]}"#,
    r#"[
        {"id":"call","name":"tool","arguments":{"a":1}},
        {"id":"call__concat_2","name":"tool","arguments":{"a":2}},
        {"id":"call__concat_1","name":"tool","arguments":{}}
    ]"#
)]
#[case::sanitized_collision_skips_the_suffix(
    r#"{"choices":[{"message":{"tool_calls":[
        {"id":"a b","function":{"name":"tool","arguments":"{\"a\":1}{\"a\":2}"}},
        {"id":"a_b__concat_1","function":{"name":"tool","arguments":"{}"}}
    ]}}]}"#,
    r#"[
        {"id":"a b","name":"tool","arguments":{"a":1}},
        {"id":"a b__concat_2","name":"tool","arguments":{"a":2}},
        {"id":"a_b__concat_1","name":"tool","arguments":{}}
    ]"#
)]
#[case::without_id_every_call_is_null(
    r#"{"choices":[{"message":{"tool_calls":[
        {"function":{"name":"tool","arguments":"{\"a\":1}{\"a\":2}"}}
    ]}}]}"#,
    r#"[
        {"id":null,"name":"tool","arguments":{"a":1}},
        {"id":null,"name":"tool","arguments":{"a":2}}
    ]"#
)]
#[case::repeated_ids_share_the_suffix_counter(
    r#"{"choices":[{"message":{"tool_calls":[
        {"id":"dup","function":{"name":"tool","arguments":"{\"a\":1}{\"a\":2}"}},
        {"id":"dup","function":{"name":"tool","arguments":"{\"a\":1}{\"a\":2}"}}
    ]}}]}"#,
    r#"[
        {"id":"dup","name":"tool","arguments":{"a":1}},
        {"id":"dup__concat_1","name":"tool","arguments":{"a":2}},
        {"id":"dup","name":"tool","arguments":{"a":1}},
        {"id":"dup__concat_2","name":"tool","arguments":{"a":2}}
    ]"#
)]
#[case::single_argument_calls_keep_their_ids(
    r#"{"choices":[{"message":{"tool_calls":[
        {"id":"x","function":{"name":"tool","arguments":"{}"}},
        {"id":"y","function":{"name":"tool","arguments":"{}"}}
    ]}}]}"#,
    r#"[
        {"id":"x","name":"tool","arguments":{}},
        {"id":"y","name":"tool","arguments":{}}
    ]"#
)]
fn response_tool_calls_behavior_tests_behavior(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(
        chat_completion_tool_calls(&response(json), false),
        calls(expected)
    );
}
