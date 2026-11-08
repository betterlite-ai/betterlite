#![cfg(test)]

use super::support::{attr, observation_attributes};

#[test]
fn writes_the_role_and_content_without_tool_calls() {
    assert_eq!(
        observation_attributes(
            r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"{"content":"hello","role":"assistant"}"#
        )]
    );
}

#[test]
fn skips_an_empty_role_and_a_null_content() {
    assert!(
        observation_attributes(r#"{"choices":[{"message":{"role":"","content":null}}]}"#)
            .is_empty()
    );
}

#[test]
fn writes_nothing_for_an_empty_choice_message() {
    assert!(observation_attributes(r#"{"choices":[{}]}"#).is_empty());
}
