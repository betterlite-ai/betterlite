#![cfg(test)]

use super::support::{attr, observation_attributes};

#[test]
fn writes_function_calls_with_parsed_arguments() {
    assert_eq!(
        observation_attributes(
            r#"{"output":[{"type":"function_call","id":"fc-1","name":"lookup","call_id":"call-1","arguments":"{\"city\": \"Paris\"}"}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{"city":"Paris"},"call_id":"call-1","id":"fc-1","name":"lookup","type":"function_call"}]"#
        )]
    );
}

#[test]
fn falls_back_to_an_empty_object_for_invalid_arguments() {
    assert_eq!(
        observation_attributes(r#"{"output":[{"type":"function_call","arguments":"not json"}]}"#),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"arguments":{},"call_id":"","id":"","name":"","type":"function_call"}]"#
        )]
    );
}
