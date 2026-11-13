use serde::Deserialize;

use super::content::{
    ConvertedPart, SamplingContent, SamplingRole, convert_content, extract_tool_calls,
    extract_tool_results, has_tool_result, has_tool_use, sampling_content, text_parts,
};
use super::openai_wire::{OpenAiContent, OpenAiMessage, OpenAiRole};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SamplingMessage {
    pub role: SamplingRole,
    #[serde(deserialize_with = "sampling_content")]
    pub content: SamplingContent,
}

#[must_use]
pub fn convert_messages(
    messages: &[SamplingMessage],
    system_prompt: Option<&str>,
) -> Vec<OpenAiMessage> {
    let mut converted = Vec::with_capacity(messages.len().saturating_add(1));
    if let Some(prompt) = system_prompt.filter(|prompt| !prompt.is_empty()) {
        converted.push(OpenAiMessage {
            role: OpenAiRole::System,
            content: Some(OpenAiContent::Text(prompt.to_owned())),
            tool_calls: None,
            tool_call_id: None,
        });
    }
    for message in messages {
        convert_message(message, &mut converted);
    }
    converted
}

fn convert_message(message: &SamplingMessage, converted: &mut Vec<OpenAiMessage>) {
    if message.role == SamplingRole::Assistant && has_tool_use(&message.content) {
        converted.push(OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: text_parts(&message.content).map(OpenAiContent::Text),
            tool_calls: Some(extract_tool_calls(&message.content)),
            tool_call_id: None,
        });
        return;
    }
    if message.role == SamplingRole::User && has_tool_result(&message.content) {
        converted.extend(extract_tool_results(&message.content));
        return;
    }
    append_regular(message, converted);
}

fn append_regular(message: &SamplingMessage, converted: &mut Vec<OpenAiMessage>) {
    let mut regular_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();
    for part in convert_content(&message.content) {
        match part {
            ConvertedPart::Regular(content_part) => regular_parts.push(content_part),
            ConvertedPart::ToolUse(call) => tool_calls.push(call),
            ConvertedPart::ToolResult(result) => tool_results.push(result),
        }
    }
    if tool_calls.is_empty() {
        if !regular_parts.is_empty() {
            converted.push(OpenAiMessage {
                role: OpenAiRole::from(message.role),
                content: Some(OpenAiContent::Parts(regular_parts)),
                tool_calls: None,
                tool_call_id: None,
            });
        }
    } else {
        converted.push(OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: (!regular_parts.is_empty()).then_some(OpenAiContent::Parts(regular_parts)),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        });
    }
    converted.extend(tool_results);
}

impl From<SamplingRole> for OpenAiRole {
    fn from(role: SamplingRole) -> Self {
        match role {
            SamplingRole::User => Self::User,
            SamplingRole::Assistant => Self::Assistant,
        }
    }
}
