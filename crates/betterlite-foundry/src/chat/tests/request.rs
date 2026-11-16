use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::chat::request::transform_request;

use super::support::{body, messages, object};

const MESSAGES: &str =
    r#"[{"role":"assistant","content":[{"type":"text","text":"a"}],"reasoning_content":"r"}]"#;

fn request(params: &str) -> Json {
    let optional = object(params);
    let transformed = transform_request("fixture-model", &messages(MESSAGES), &optional)
        .expect("the request transforms");
    let encoded = serde_json::to_value(transformed).expect("the request body encodes");
    serde_json::from_value(encoded).expect("the request body reads as JSON")
}

pub(super) fn extra_body_precedence() {
    // the merged keys overwrite existing options, and max_retries leaves with them
    assert_eq!(
        request(
            r#"{"temperature":1,"extra_body":{"temperature":0.2,"top_p":0.9,"max_retries":3}}"#
        ),
        body(
            r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}],
                "temperature":0.2,"top_p":0.9}"#
        )
    );
    assert_eq!(
        request(r#"{"temperature":9,"extra_body":{"temperature":1}}"#),
        body(
            r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}],"temperature":1}"#
        )
    );
    assert_eq!(
        request(r#"{"max_retries":5,"n":2}"#),
        body(r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}],"n":2}"#)
    );
    assert_eq!(
        request("{}"),
        body(r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}]}"#)
    );
}

#[rstest]
#[case::empty_object(
    r#"{"extra_body":{}}"#,
    r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}]}"#
)]
#[case::string(
    r#"{"extra_body":"x","seed":1}"#,
    r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}],"seed":1}"#
)]
#[case::list(
    r#"{"extra_body":[],"seed":1}"#,
    r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}],"seed":1}"#
)]
#[case::null(
    r#"{"extra_body":null,"seed":1}"#,
    r#"{"model":"fixture-model","messages":[{"role":"assistant","content":"a"}],"seed":1}"#
)]
fn extra_body_is_dropped_unless_it_is_a_non_empty_object(
    #[case] params: &str,
    #[case] expected: &str,
) {
    assert_eq!(request(params), body(expected));
}

#[test]
fn request_keeps_an_option_that_shadows_the_message_list() {
    assert_eq!(
        request(r#"{"messages":[{"role":"user","content":"shadow"}]}"#),
        body(r#"{"model":"fixture-model","messages":[{"role":"user","content":"shadow"}]}"#)
    );
}

#[test]
fn request_does_not_mutate_the_caller_options() {
    let optional = object(r#"{"extra_body":{"top_p":0.9},"max_retries":3}"#);
    let untouched = optional.clone();
    transform_request("fixture-model", &messages(MESSAGES), &optional).expect("the request body");
    assert_eq!(optional, untouched);
}
