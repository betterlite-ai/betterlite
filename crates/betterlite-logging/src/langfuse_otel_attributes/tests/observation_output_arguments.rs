#![cfg(test)]

use super::support::{attr, observation_attributes};

#[test]
fn parses_text_arguments_into_json() {
    assert_eq!(
        observation_attributes(
            r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{\"count\": 2}"}}]}}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{"count":2},"call_id":"","id":"","name":"","type":"function_call"}]"#
        )]
    );
}

#[test]
fn falls_back_to_an_empty_object_for_invalid_text() {
    assert_eq!(
        observation_attributes(
            r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":"not json"}}]}}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{},"call_id":"","id":"","name":"","type":"function_call"}]"#
        )]
    );
}

#[test]
fn keeps_arguments_that_are_not_text() {
    assert_eq!(
        observation_attributes(
            r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":{"count": 2}}}]}}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{"count":2},"call_id":"","id":"","name":"","type":"function_call"}]"#
        )]
    );
}
