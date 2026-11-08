#![cfg(test)]

use super::support::{attr, messages_attributes};

#[test]
fn writes_the_messages_as_observation_input() {
    assert_eq!(
        messages_attributes(r#"{"messages":[{"role":"user","content":"hi"}]}"#),
        vec![attr(
            "langfuse.observation.input",
            r#"{"messages":[{"content":"hi","role":"user"}]}"#
        )]
    );
}

#[test]
fn adds_functions_and_tools_from_optional_params() {
    assert_eq!(
        messages_attributes(
            r#"{"messages":[],"optional_params":{"functions":[{"name":"f"}],"tools":[{"type":"function"}]}}"#
        ),
        vec![attr(
            "langfuse.observation.input",
            r#"{"messages":[],"functions":[{"name":"f"}],"tools":[{"type":"function"}]}"#
        )]
    );
}

#[test]
fn skips_null_functions_and_tools() {
    assert_eq!(
        messages_attributes(
            r#"{"messages":null,"optional_params":{"functions":null,"tools":null}}"#
        ),
        vec![attr("langfuse.observation.input", r#"{"messages":null}"#)]
    );
}

#[test]
fn writes_null_messages_when_absent() {
    assert_eq!(
        messages_attributes("{}"),
        vec![attr("langfuse.observation.input", r#"{"messages":null}"#)]
    );
}
