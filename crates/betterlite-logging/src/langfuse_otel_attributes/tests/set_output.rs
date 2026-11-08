#![cfg(test)]

use super::support::{attr, response_attributes};

#[test]
fn writes_an_object_response_as_json() {
    assert_eq!(
        response_attributes(r#"{"id":"resp-1","output":[]}"#),
        vec![attr(
            "langfuse.observation.output",
            r#"{"id":"resp-1","output":[]}"#
        )]
    );
}

#[test]
fn writes_a_list_response_as_json() {
    assert_eq!(
        response_attributes(r#"[{"beta":1,"alpha":2}]"#),
        vec![attr(
            "langfuse.observation.output",
            r#"[{"alpha":2,"beta":1}]"#
        )]
    );
}

#[test]
fn writes_the_empty_string_for_a_null_response() {
    assert_eq!(
        response_attributes("null"),
        vec![attr("langfuse.observation.output", "")]
    );
}

#[test]
fn writes_the_empty_string_for_an_empty_object() {
    assert_eq!(
        response_attributes("{}"),
        vec![attr("langfuse.observation.output", "")]
    );
}
