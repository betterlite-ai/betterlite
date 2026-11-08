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

#[test]
fn falls_back_to_an_empty_object_for_text_over_one_mebibyte() {
    let arguments = format!(r#"{{"pad":"{}"}}"#, "a".repeat(1_100_000));
    let escaped = serde_json::to_string(&arguments).unwrap();
    let response =
        r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":ESCAPED}}]}}]}"#
            .replace("ESCAPED", &escaped);
    assert_eq!(
        observation_attributes(&response),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{},"call_id":"","id":"","name":"","type":"function_call"}]"#
        )]
    );
}
