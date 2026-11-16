#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::apply_guardrailed_tool_calls;
use crate::guardrails::chat_guardrail_translation::rewrites::error::{InputError, RewriteError};
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{object, values};

const MESSAGES: &str = r#"[
    {"role": "assistant", "tool_calls": [
        {"id": "1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
        {"id": "2", "type": "function", "function": {"name": "g", "arguments": "{}"}}]},
    {"role": "assistant", "tool_calls": [
        {"id": "3", "type": "function", "function": {"name": "h", "arguments": "{}"}}]}]"#;

fn call(source: &str) -> GuardrailValue {
    GuardrailValue::Object(object(source))
}

#[test]
fn apply_input_tool_replaces_each_call_by_message_and_call_index() {
    let mut messages = values(MESSAGES);
    let returned = vec![
        call(r#"{"id": "7", "function": {"name": "x"}}"#),
        call(r#"{"id": "8"}"#),
    ];
    apply_guardrailed_tool_calls(&mut messages, returned, &[(1, 0), (0, 1)]).unwrap();
    let expected = values(
        r#"[
    {"role": "assistant", "tool_calls": [
        {"id": "1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
        {"id": "8"}]},
    {"role": "assistant", "tool_calls": [
        {"id": "7", "function": {"name": "x"}}]}]"#,
    );
    assert_eq!(messages, expected);
}

#[test]
fn apply_input_tool_leaves_the_messages_alone_when_the_guardrail_returned_no_calls() {
    let mut messages = values(MESSAGES);
    apply_guardrailed_tool_calls(&mut messages, Vec::new(), &[(0, 0)]).unwrap();
    assert_eq!(messages, values(MESSAGES));
}

#[test]
fn apply_input_tool_keeps_the_calls_the_guardrail_did_not_return() {
    let mut messages = values(MESSAGES);
    let returned = vec![call(r#"{"id": "7"}"#)];
    apply_guardrailed_tool_calls(&mut messages, returned, &[(0, 0), (0, 1)]).unwrap();
    let expected = MESSAGES.replacen(
        r#"{"id": "1", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#,
        r#"{"id": "7"}"#,
        1,
    );
    assert_eq!(messages, values(&expected));
}

#[test]
fn apply_input_tool_refuses_a_returned_call_that_is_not_an_object() {
    let mut messages = values(MESSAGES);
    let returned = vec![call(r#"{"id": "7"}"#), GuardrailValue::Text("x".to_owned())];
    let failure =
        apply_guardrailed_tool_calls(&mut messages, returned, &[(0, 0), (0, 1)]).unwrap_err();
    assert!(matches!(
        failure,
        InputError::Rewrite(RewriteError::ToolCallNotObject { task_index: 1 })
    ));
    assert_eq!(messages, values(MESSAGES));
}
