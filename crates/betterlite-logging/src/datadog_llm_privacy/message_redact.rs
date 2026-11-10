use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::arguments::ToolArguments;
use crate::datadog_llm_obs::constants::{SAFE_REDACTED_MESSAGE_ROLES, safe_message_role};
use crate::datadog_llm_obs::records::{DdMessage, Message, ToolCall, ToolResult};
use crate::exporter_options::redaction::REDACTED_TEXT;

// LiteLLM replaces every message's content and every tool argument or result, keeping only the identifiers
#[must_use]
pub fn redact_messages(messages: &[DdMessage]) -> Vec<Message> {
    let mut redacted = Vec::with_capacity(messages.len());
    for message in messages {
        redacted.push(redacted_message(message));
    }
    redacted
}

fn redacted_message(message: &DdMessage) -> Message {
    let (role, tool_calls, tool_results) = match message {
        DdMessage::Message(fields) => (
            safe_role(&fields.role),
            redacted_calls(&fields.tool_calls),
            redacted_results(&fields.tool_results),
        ),
        DdMessage::Input(_input) => (String::new(), Vec::new(), Vec::new()),
        DdMessage::Raw(fields) => (
            safe_message_role(fields.get("role")),
            Vec::new(),
            Vec::new(),
        ),
    };
    Message {
        role,
        content: Json::Text(REDACTED_TEXT.to_owned()),
        reasoning_content: None,
        tool_calls,
        tool_results,
    }
}

fn safe_role(role: &str) -> String {
    if SAFE_REDACTED_MESSAGE_ROLES.contains(&role) {
        return role.to_owned();
    }
    String::new()
}

fn redacted_calls(calls: &[ToolCall]) -> Vec<ToolCall> {
    calls
        .iter()
        .map(|call| ToolCall {
            name: call.name.clone(),
            arguments: ToolArguments::Text(REDACTED_TEXT.to_owned()),
            tool_id: call.tool_id.clone(),
            kind: call.kind.clone(),
        })
        .collect()
}

fn redacted_results(results: &[ToolResult]) -> Vec<ToolResult> {
    results
        .iter()
        .map(|result| ToolResult {
            name: result.name.clone(),
            result: REDACTED_TEXT.to_owned(),
            tool_id: result.tool_id.clone(),
            kind: result.kind.clone(),
        })
        .collect()
}

mod tests;
