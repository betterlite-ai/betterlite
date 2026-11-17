#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::{PermissionError, mapping_value, object_list};
use crate::tool_permission_responses::evaluate::DeniedToolCall;
use crate::tool_permission_responses::extraction::ToolCallInfo;
use crate::tool_permission_responses::tests::support::object;

use super::rewrite_anthropic_responses;

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
            rule_id: None,
            message: format!("Tool '{name}' denied by default action"),
        },
    }
}

fn content_blocks(response: &Object) -> &[GuardrailValue] {
    response
        .get("content")
        .and_then(object_list)
        .expect("content")
}

fn text_of(block: &GuardrailValue) -> Option<&str> {
    match mapping_value(block, "text") {
        Some(GuardrailValue::Text(text)) => Some(text),
        None | Some(_) => None,
    }
}

#[test]
fn rewrite_anthropic_responses_removes_a_denied_block_and_appends_the_error() {
    let mut response = object(
        r#"{"stop_reason":"tool_use","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"shell","input":{}}]}"#,
    );
    rewrite_anthropic_responses(&mut response, &[denied("t1", "shell")]);
    let blocks = content_blocks(&response);
    assert_eq!(blocks.len(), 2);
    assert_eq!(text_of(&blocks[0]), Some("hi"));
    assert_eq!(
        text_of(&blocks[1]),
        Some("Permission denied: Tool 'shell' denied by default action")
    );
    assert_eq!(
        response.get("stop_reason"),
        Some(&GuardrailValue::Text("end_turn".to_owned()))
    );
}

#[test]
fn rewrite_anthropic_responses_keeps_the_stop_reason_when_a_tool_use_remains() {
    let mut response = object(
        r#"{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"shell","input":{}},{"type":"tool_use","id":"t2","name":"read","input":{}}]}"#,
    );
    rewrite_anthropic_responses(&mut response, &[denied("t1", "shell")]);
    assert_eq!(content_blocks(&response).len(), 2);
    assert_eq!(
        response.get("stop_reason"),
        Some(&GuardrailValue::Text("tool_use".to_owned()))
    );
}

#[test]
fn rewrite_anthropic_responses_keeps_a_tool_use_block_without_an_id() {
    let mut response = object(
        r#"{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"shell","input":{}}]}"#,
    );
    rewrite_anthropic_responses(&mut response, &[denied("", "shell")]);
    let blocks = content_blocks(&response);
    assert_eq!(blocks.len(), 2);
    assert_eq!(text_of(&blocks[1]), Some(""));
    assert_eq!(
        response.get("stop_reason"),
        Some(&GuardrailValue::Text("tool_use".to_owned()))
    );
}

#[test]
fn rewrite_anthropic_responses_keeps_a_non_tool_block_whose_id_matches() {
    let mut response = object(
        r#"{"stop_reason":"tool_use","content":[{"type":"text","id":"t1","text":"hi"},{"type":"tool_use","id":"t1","name":"shell","input":{}}]}"#,
    );
    rewrite_anthropic_responses(&mut response, &[denied("t1", "shell")]);
    let blocks = content_blocks(&response);
    assert_eq!(blocks.len(), 2);
    assert_eq!(text_of(&blocks[0]), Some("hi"));
}

#[test]
fn rewrite_anthropic_responses_ignores_a_non_text_id() {
    let mut response =
        object(r#"{"content":[{"type":"tool_use","id":5,"name":"shell","input":{}}]}"#);
    rewrite_anthropic_responses(&mut response, &[denied("5", "shell")]);
    assert_eq!(content_blocks(&response).len(), 2);
}

#[test]
fn rewrite_anthropic_responses_adds_the_text_block_for_an_unmatched_denial() {
    let mut response = object(r#"{"content":[{"type":"text","text":"hi"}]}"#);
    rewrite_anthropic_responses(&mut response, &[denied("gone", "shell")]);
    let blocks = content_blocks(&response);
    assert_eq!(blocks.len(), 2);
    assert_eq!(text_of(&blocks[1]), Some(""));
    assert_eq!(
        response.get("stop_reason"),
        Some(&GuardrailValue::Text("end_turn".to_owned()))
    );
}

#[test]
fn rewrite_anthropic_responses_does_nothing_without_denials() {
    let mut response = object(r#"{"content":[{"type":"tool_use","id":"t1","name":"shell"}]}"#);
    let before = response.clone();
    rewrite_anthropic_responses(&mut response, &[]);
    assert_eq!(response, before);
}

#[test]
fn rewrite_anthropic_responses_ignores_a_response_without_a_content_list() {
    let mut response = object(r#"{"content":"nope"}"#);
    let before = response.clone();
    rewrite_anthropic_responses(&mut response, &[denied("t1", "shell")]);
    assert_eq!(response, before);
}
