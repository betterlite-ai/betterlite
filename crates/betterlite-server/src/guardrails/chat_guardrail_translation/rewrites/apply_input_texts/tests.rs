#![cfg(test)]

use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;

use super::apply_guardrailed_texts;
use crate::guardrails::chat_guardrail_translation::rewrites::error::InputError;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::values;

const MESSAGES: &str = r#"[
    {"role": "user", "content": "hello"},
    {"role": "user", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}]"#;

fn texts(responses: &[&str]) -> Vec<String> {
    responses.iter().map(|text| (*text).to_owned()).collect()
}

#[test]
fn apply_input_text_writes_string_and_list_content_by_message_and_content_index() {
    let mut messages = values(MESSAGES);
    let mappings = [(0, None), (1, Some(0)), (1, Some(1))];
    let sent = texts(&["hello", "a", "b"]);
    let returned = texts(&["X", "Y", "Z"]);
    apply_guardrailed_texts(&mut messages, &returned, &sent, &mappings, Some("g")).unwrap();
    let expected = MESSAGES
        .replace(r#""hello""#, r#""X""#)
        .replace(r#""text": "a""#, r#""text": "Y""#)
        .replace(r#""text": "b""#, r#""text": "Z""#);
    assert_eq!(messages, values(&expected));
}

#[test]
fn apply_input_text_leaves_a_message_alone_when_the_guardrail_returned_no_text() {
    let mut messages = values(MESSAGES);
    let sent = texts(&["hello"]);
    apply_guardrailed_texts(&mut messages, &[], &sent, &[(0, None)], None).unwrap();
    assert_eq!(messages, values(MESSAGES));
}

#[test]
fn apply_input_text_does_nothing_when_no_text_was_sent() {
    let mut messages = values(MESSAGES);
    apply_guardrailed_texts(&mut messages, &texts(&["X"]), &[], &[], None).unwrap();
    assert_eq!(messages, values(MESSAGES));
}

#[test]
fn apply_input_text_refuses_a_different_number_of_texts_and_names_the_guardrail() {
    let mut messages = values(MESSAGES);
    let sent = texts(&["hello", "a"]);
    let mappings = [(0, None), (1, Some(0))];
    let failure =
        apply_guardrailed_texts(&mut messages, &texts(&["X"]), &sent, &mappings, Some("pii"))
            .unwrap_err();
    assert!(matches!(
        failure,
        InputError::Unappliable(UnappliableRequestRewrite { guardrail_name }) if guardrail_name == "pii"
    ));
    assert_eq!(messages, values(MESSAGES));
}

#[test]
fn apply_input_text_names_an_unknown_guardrail_when_it_has_no_name() {
    let mut messages = values(MESSAGES);
    let sent = texts(&["hello"]);
    let failure = apply_guardrailed_texts(
        &mut messages,
        &texts(&["X", "Y"]),
        &sent,
        &[(0, None)],
        None,
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        InputError::Unappliable(UnappliableRequestRewrite { guardrail_name }) if guardrail_name == "unknown"
    ));
}
