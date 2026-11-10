#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::arguments::ToolArguments;
use crate::datadog_llm_obs::records::{DdMessage, InputMessage, Message, ToolCall, ToolResult};
use crate::datadog_llm_privacy::message_redact::redact_messages;

const REDACTED: &str = "redacted-by-litellm";

fn message(role: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Json::Text("the prompt".to_owned()),
        reasoning_content: Some(Json::Text("the reasoning".to_owned())),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    }
}

#[test]
fn message_redaction_keeps_the_role_and_replaces_the_content() {
    let redacted = redact_messages(&[DdMessage::Message(message("assistant"))]);
    assert_eq!(redacted.len(), 1);
    assert_eq!(redacted[0].role, "assistant");
    assert_eq!(redacted[0].content, Json::Text(REDACTED.to_owned()));
    assert_eq!(redacted[0].reasoning_content, None);
}

#[test]
fn message_redaction_drops_a_role_outside_the_safe_set() {
    let redacted = redact_messages(&[DdMessage::Message(message("hooligan"))]);
    assert_eq!(redacted[0].role, "");
}

#[test]
fn message_redaction_redacts_tool_calls_and_keeps_their_identity() {
    let mut source = message("assistant");
    source.tool_calls = vec![ToolCall {
        name: "get_weather".to_owned(),
        arguments: ToolArguments::Text("{\"city\": \"Paris\"}".to_owned()),
        tool_id: "call_1".to_owned(),
        kind: "function".to_owned(),
    }];
    let redacted = redact_messages(&[DdMessage::Message(source)]);
    assert_eq!(
        redacted[0].tool_calls,
        vec![ToolCall {
            name: "get_weather".to_owned(),
            arguments: ToolArguments::Text(REDACTED.to_owned()),
            tool_id: "call_1".to_owned(),
            kind: "function".to_owned(),
        }]
    );
}

#[test]
fn message_redaction_redacts_tool_results_and_keeps_their_identity() {
    let mut source = message("tool");
    source.tool_results = vec![ToolResult {
        name: "get_weather".to_owned(),
        result: "{\"temperature\": 21}".to_owned(),
        tool_id: "call_1".to_owned(),
        kind: "function".to_owned(),
    }];
    let redacted = redact_messages(&[DdMessage::Message(source)]);
    assert_eq!(
        redacted[0].tool_results,
        vec![ToolResult {
            name: "get_weather".to_owned(),
            result: REDACTED.to_owned(),
            tool_id: "call_1".to_owned(),
            kind: "function".to_owned(),
        }]
    );
}

#[test]
fn message_redaction_empties_a_message_that_is_not_an_object() {
    let input = DdMessage::Input(InputMessage {
        input: "a bare string".to_owned(),
    });
    let redacted = redact_messages(&[input]);
    assert_eq!(redacted[0].role, "");
    assert_eq!(redacted[0].content, Json::Text(REDACTED.to_owned()));
}
