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
#[case::sanitized_ids_keep_their_generators_apart(
    r#"{"choices":[{"message":{"tool_calls":[
        {"id":"a_b","function":{"name":"tool","arguments":"{\"a\":1}{\"a\":2}"}},
        {"id":"a b","function":{"name":"tool","arguments":"{}"}}
    ]}}]}"#,
    r#"[
        {"id":"a_b","name":"tool","arguments":{"a":1}},
        {"id":"a_b__concat_1","name":"tool","arguments":{"a":2}},
        {"id":"a b","name":"tool","arguments":{}}
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

fn many_concatenated_calls(count: usize) -> String {
    let mut entries = Vec::new();
    for index in 0..count {
        entries.push(format!(
            r#"{{"id":"call_{index}","function":{{"name":"tool","arguments":"{{\"n\":1}}{{\"n\":2}}"}}}}"#
        ));
    }
    format!(
        r#"{{"choices":[{{"message":{{"tool_calls":[{}]}}}}]}}"#,
        entries.join(",")
    )
}

#[test]
fn response_tool_calls_many_concatenated_calls_behavior() {
    let computed = chat_completion_tool_calls(&response(&many_concatenated_calls(1000)), false);
    assert_eq!(computed.len(), 2000);
    assert_eq!(
        computed.first().and_then(|call| call.id.as_deref()),
        Some("call_0")
    );
    assert_eq!(
        computed.get(1).and_then(|call| call.id.as_deref()),
        Some("call_0__concat_1")
    );
    assert_eq!(
        computed.get(1998).and_then(|call| call.id.as_deref()),
        Some("call_999")
    );
    assert_eq!(
        computed.last().and_then(|call| call.id.as_deref()),
        Some("call_999__concat_1")
    );
    assert!(
        computed
            .iter()
            .all(|call| call.name.as_deref() == Some("tool"))
    );
}
