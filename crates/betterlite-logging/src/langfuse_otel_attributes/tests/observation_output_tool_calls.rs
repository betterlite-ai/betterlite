#![cfg(test)]

use super::support::{attr, observation_attributes};

#[test]
fn writes_transformed_tool_calls() {
    assert_eq!(
        observation_attributes(
            r#"{"id":"resp-1","choices":[{"message":{"tool_calls":[{"id":"call-1","function":{"name":"lookup","arguments":"{\"city\": \"Paris\"}"}}]}}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{"city":"Paris"},"call_id":"call-1","id":"resp-1","name":"lookup","type":"function_call"}]"#
        )]
    );
}

#[test]
fn missing_ids_and_names_default_to_the_empty_string() {
    assert_eq!(
        observation_attributes(r#"{"choices":[{"message":{"tool_calls":[{"function":{}}]}}]}"#),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{},"call_id":"","id":"","name":"","type":"function_call"}]"#
        )]
    );
}
