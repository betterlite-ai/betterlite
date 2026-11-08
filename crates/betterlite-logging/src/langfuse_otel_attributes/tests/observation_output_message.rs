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

#[test]
fn empty_tool_calls_fall_through_to_the_role_and_content() {
    assert_eq!(
        observation_attributes(
            r#"{"choices":[{"message":{"role":"assistant","content":"hi","tool_calls":[]}}]}"#
        ),
        vec![attr(
            "langfuse.observation.output",
            r#"{"content":"hi","role":"assistant"}"#
        )]
    );
}

#[test]
fn writes_an_empty_content_when_it_is_present() {
    assert_eq!(
        observation_attributes(r#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#),
        vec![attr(
            "langfuse.observation.output",
            r#"{"content":"","role":"assistant"}"#
        )]
    );
}
