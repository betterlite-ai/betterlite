#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::{mapping_value, object_list};
use crate::tool_permission_responses::tests::support::{
    GUARDRAIL_NAME, guardrail, object, rewrite_guardrail, rule, skipping_guardrail, text,
};

use super::{ResponseFormat, post_call_success_hook};

fn run(
    guardrail: &crate::tool_permission::ToolPermissionGuardrail,
    data: &mut Object,
    format: ResponseFormat,
    response: &mut GuardrailValue,
) -> Result<(), GuardrailFailure> {
    post_call_success_hook(guardrail, data, format, &BTreeSet::new(), response)
}

fn applied(data: &Object) -> Vec<String> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(names)) = metadata.get("applied_guardrails") else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|name| match name {
            GuardrailValue::Text(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn post_call_success_hook_skips_a_guardrail_that_should_not_run() {
    let guardrail = skipping_guardrail();
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    ));
    let before = response.clone();
    run(&guardrail, &mut data, ResponseFormat::OpenAi, &mut response).expect("no check runs");
    assert_eq!(response, before);
    assert!(data.is_empty());
}

#[test]
fn post_call_success_hook_passes_a_response_without_tool_calls() {
    let guardrail = guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response =
        GuardrailValue::Object(object(r#"{"choices":[{"message":{"content":"hi"}}]}"#));
    let before = response.clone();
    run(&guardrail, &mut data, ResponseFormat::OpenAi, &mut response).expect("nothing to check");
    assert_eq!(response, before);
    assert!(data.is_empty());
}

#[test]
fn post_call_success_hook_rewrites_a_denied_tool_call() {
    let guardrail = rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object(
        r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    ));
    run(&guardrail, &mut data, ResponseFormat::OpenAi, &mut response).expect("rewrite mode passes");
    let GuardrailValue::Object(fields) = &response else {
        panic!("response stays an object");
    };
    let choice = fields
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(mapping_value(choice, "finish_reason"), Some(&text("stop")));
    let message = mapping_value(choice, "message").expect("message");
    assert_eq!(
        mapping_value(message, "tool_calls"),
        Some(&GuardrailValue::Null)
    );
    assert_eq!(
        mapping_value(message, "content"),
        Some(&text(
            "Permission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)"
        ))
    );
    assert_eq!(applied(&data), vec![GUARDRAIL_NAME.to_owned()]);
}

#[test]
fn post_call_success_hook_raises_in_block_mode() {
    let guardrail = guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    ));
    let error = run(&guardrail, &mut data, ResponseFormat::OpenAi, &mut response)
        .expect_err("block mode raises");
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        ..
    } = error
    else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(message, "Tool 'shell' denied by rule 'block-shell'");
    assert!(blocked_content);
}

#[test]
fn post_call_success_hook_records_the_guardrail_when_a_tool_is_allowed() {
    let guardrail = guardrail(vec![rule("allow-shell", "allow", "shell")]);
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    ));
    let before = response.clone();
    run(&guardrail, &mut data, ResponseFormat::OpenAi, &mut response).expect("tool allowed");
    assert_eq!(response, before);
    assert_eq!(applied(&data), vec![GUARDRAIL_NAME.to_owned()]);
}

#[test]
fn post_call_success_hook_rewrites_anthropic_tool_use_blocks() {
    let guardrail = rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object(
        r#"{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_1","name":"shell","input":{}}]}"#,
    ));
    run(
        &guardrail,
        &mut data,
        ResponseFormat::Anthropic,
        &mut response,
    )
    .expect("rewrite mode passes");
    let GuardrailValue::Object(fields) = &response else {
        panic!("response stays an object");
    };
    assert_eq!(fields.get("stop_reason"), Some(&text("end_turn")));
    let content = fields
        .get("content")
        .and_then(object_list)
        .expect("content");
    assert_eq!(content.len(), 1);
    assert_eq!(
        mapping_value(content.first().expect("block"), "text"),
        Some(&text(
            "Permission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)"
        ))
    );
    assert_eq!(applied(&data), vec![GUARDRAIL_NAME.to_owned()]);
}

#[test]
fn post_call_success_hook_checks_an_anthropic_response_only_through_named_tool_use_blocks() {
    let guardrail = rewrite_guardrail(vec![rule("allow-read", "allow", "read")]);
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object(
        r#"{"content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"read"},{"type":"tool_use","id":"t2"}]}"#,
    ));
    let before = response.clone();
    run(
        &guardrail,
        &mut data,
        ResponseFormat::Anthropic,
        &mut response,
    )
    .expect("read is allowed");
    assert_eq!(response, before);
    assert_eq!(applied(&data), vec![GUARDRAIL_NAME.to_owned()]);
}

#[test]
fn post_call_success_hook_passes_a_non_object_openai_response() {
    let guardrail = guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response = text("not a response");
    run(&guardrail, &mut data, ResponseFormat::OpenAi, &mut response).expect("shape check skips");
    assert_eq!(response, text("not a response"));
    assert!(data.is_empty());
}

#[test]
fn post_call_success_hook_passes_an_anthropic_body_without_content_blocks() {
    let guardrail = guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response = GuardrailValue::Object(object("{}"));
    let before = response.clone();
    run(
        &guardrail,
        &mut data,
        ResponseFormat::Anthropic,
        &mut response,
    )
    .expect("no content blocks skips");
    assert_eq!(response, before);
    assert!(data.is_empty());
}

#[test]
fn post_call_success_hook_passes_an_anthropic_response_without_tool_use() {
    let guardrail = guardrail(vec![rule("block-shell", "deny", "shell")]);
    let mut data = Object::new();
    let mut response =
        GuardrailValue::Object(object(r#"{"content":[{"type":"text","text":"hi"}]}"#));
    let before = response.clone();
    run(
        &guardrail,
        &mut data,
        ResponseFormat::Anthropic,
        &mut response,
    )
    .expect("no tool uses skips");
    assert_eq!(response, before);
    assert!(data.is_empty());
}
