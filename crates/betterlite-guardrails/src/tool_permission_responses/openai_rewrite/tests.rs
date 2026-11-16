#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::tool_permission::types::{PermissionError, mapping_value};
use crate::tool_permission_responses::tests::support::object;

use super::rewrite_openai_responses;
use crate::tool_permission_responses::evaluate::DeniedToolCall;
use crate::tool_permission_responses::extraction::ToolCallInfo;

fn denied(id: &str, name: &str) -> DeniedToolCall {
    DeniedToolCall {
        call: ToolCallInfo {
            id: id.to_owned(),
            kind: Some("function".to_owned()),
            name: Some(name.to_owned()),
            arguments: None,
        },
        error: PermissionError {
            tool_name: name.to_owned(),
            rule_id: Some(format!("block-{name}")),
            message: format!("Tool '{name}' denied by rule 'block-{name}'"),
        },
    }
}

fn text_of(value: Option<&GuardrailValue>) -> Option<&str> {
    match value {
        Some(GuardrailValue::Text(text)) => Some(text),
        None | Some(_) => None,
    }
}

fn message(response: &crate::guardrail_message_scope::Object) -> &GuardrailValue {
    mapping_value(
        response
            .get("choices")
            .and_then(crate::tool_permission::types::object_list)
            .expect("choices")
            .first()
            .expect("choice"),
        "message",
    )
    .expect("message")
}

#[test]
fn rewrite_openai_responses_removes_a_denied_call_and_keeps_the_finish_reason() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}},{"id":"c2","function":{"name":"read"}}]}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    let choice = response
        .get("choices")
        .and_then(crate::tool_permission::types::object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(
        text_of(mapping_value(choice, "finish_reason")),
        Some("tool_calls")
    );
    let message = message(&response);
    let kept = mapping_value(message, "tool_calls")
        .and_then(crate::tool_permission::types::object_list)
        .expect("tool calls");
    assert_eq!(kept.len(), 1);
    assert_eq!(
        text_of(mapping_value(kept.first().expect("kept"), "id")),
        Some("c2")
    );
    assert_eq!(
        text_of(mapping_value(message, "content")),
        Some("Permission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)")
    );
}

#[test]
fn rewrite_openai_responses_stops_when_every_tool_call_is_removed() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    let choice = response
        .get("choices")
        .and_then(crate::tool_permission::types::object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(
        text_of(mapping_value(choice, "finish_reason")),
        Some("stop")
    );
    let message = message(&response);
    assert_eq!(
        mapping_value(message, "tool_calls"),
        Some(&GuardrailValue::Null)
    );
}

#[test]
fn rewrite_openai_responses_keeps_a_finish_reason_that_is_not_tool_calling() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"length","message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    let choice = response
        .get("choices")
        .and_then(crate::tool_permission::types::object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(
        text_of(mapping_value(choice, "finish_reason")),
        Some("length")
    );
}

#[test]
fn rewrite_openai_responses_appends_errors_after_existing_content() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"content":"thinking","tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    assert_eq!(
        text_of(mapping_value(message(&response), "content")),
        Some(
            "thinking\n\nPermission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)"
        )
    );
}

#[test]
fn rewrite_openai_responses_starts_the_content_when_it_is_empty() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"content":"","tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    assert_eq!(
        text_of(mapping_value(message(&response), "content")),
        Some("Permission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)")
    );
}

#[test]
fn rewrite_openai_responses_removes_a_denied_legacy_function_call() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"function_call","message":{"function_call":{"name":"shell","arguments":"{}"}}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("legacy_function_call_0", "shell")]);
    let choice = response
        .get("choices")
        .and_then(crate::tool_permission::types::object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(
        text_of(mapping_value(choice, "finish_reason")),
        Some("stop")
    );
    assert_eq!(
        mapping_value(message(&response), "function_call"),
        Some(&GuardrailValue::Null)
    );
}

#[test]
fn rewrite_openai_responses_leaves_a_legacy_call_the_denial_does_not_name() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"function_call","message":{"function_call":{"name":"shell","arguments":"{}"}}}]}"#,
    );
    rewrite_openai_responses(&mut response, &[denied("call_9", "read")]);
    let choice = response
        .get("choices")
        .and_then(crate::tool_permission::types::object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(
        text_of(mapping_value(choice, "finish_reason")),
        Some("function_call")
    );
    let message = message(&response);
    let function_call = mapping_value(message, "function_call").expect("function call");
    assert_eq!(text_of(mapping_value(function_call, "name")), Some("shell"));
    assert_eq!(mapping_value(message, "content"), None);
    assert_eq!(
        mapping_value(message, "tool_calls"),
        Some(&GuardrailValue::Null)
    );
}

#[test]
fn rewrite_openai_responses_keeps_a_tool_call_the_denial_does_not_name() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    let before = response.clone();
    rewrite_openai_responses(&mut response, &[denied("other", "read")]);
    assert_eq!(response, before);
}

#[test]
fn rewrite_openai_responses_ignores_a_response_without_choices() {
    let mut response = object("{}");
    let before = response.clone();
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    assert_eq!(response, before);
}

#[test]
fn rewrite_openai_responses_does_nothing_without_denials() {
    let mut response = object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    let before = response.clone();
    rewrite_openai_responses(&mut response, &[]);
    assert_eq!(response, before);
}

#[test]
fn rewrite_openai_responses_ignores_a_choice_that_is_not_an_object() {
    let mut response = object(r#"{"choices":[7]}"#);
    let before = response.clone();
    rewrite_openai_responses(&mut response, &[denied("c1", "shell")]);
    assert_eq!(response, before);
}
