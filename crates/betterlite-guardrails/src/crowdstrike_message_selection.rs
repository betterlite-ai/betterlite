use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_aidr::models::{GuardInput, Message, MessageContent};
use crate::crowdstrike_normalization::{extract_text_from_message, normalize_content};
use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::inputs::GuardrailApiInputs;

#[derive(Debug, Clone, PartialEq)]
pub struct FilteredMessages {
    pub messages: Vec<GuardrailMessage>,
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuardInputWithIndices {
    pub guard_input: GuardInput,
    pub sent_indices: Vec<usize>,
}

// LiteLLM's _messages_since_last_assistant: system turns survive, and the tail starts after the last assistant
#[must_use]
pub fn messages_since_last_assistant(messages: &[GuardrailMessage]) -> FilteredMessages {
    let Some(last) = messages.last() else {
        return FilteredMessages {
            messages: Vec::new(),
            indices: Vec::new(),
        };
    };
    let mut indices = Vec::new();
    if last.role == "assistant" {
        for (index, message) in messages.iter().enumerate() {
            if message.role == "system" {
                indices.push(index);
            }
        }
        indices.push(messages.len().saturating_sub(1));
    } else {
        let split = messages
            .iter()
            .rposition(|message| message.role == "assistant")
            .map_or(0, |index| index.saturating_add(1));
        for (index, message) in messages.iter().enumerate().take(split) {
            if message.role == "system" {
                indices.push(index);
            }
        }
        indices.extend(split..messages.len());
    }
    let selected = indices
        .iter()
        .filter_map(|index| messages.get(*index).cloned())
        .collect();
    FilteredMessages {
        messages: selected,
        indices,
    }
}

pub fn build_guard_input_for_request(
    inputs: &GuardrailApiInputs,
) -> Result<Option<GuardInputWithIndices>, CrowdStrikeError> {
    let structured = inputs
        .structured_messages
        .as_deref()
        .filter(|messages| !messages.is_empty());
    let texts = inputs.texts.as_deref().unwrap_or_default();
    let (mut guard_input, sent_indices) = match structured {
        Some(messages) => structured_input(messages)?,
        None if texts.is_empty() => {
            tracing::warn!(
                "CrowdStrike AIDR Guardrail: No messages or texts provided for input request"
            );
            return Ok(None);
        }
        None => texts_input(texts),
    };
    if let Some(tools) = &inputs.tools
        && !tools.is_empty()
    {
        guard_input.tools = Some(tools.clone());
    }
    Ok(Some(GuardInputWithIndices {
        guard_input,
        sent_indices,
    }))
}

fn structured_input(
    messages: &[GuardrailMessage],
) -> Result<(GuardInput, Vec<usize>), CrowdStrikeError> {
    let filtered = messages_since_last_assistant(messages);
    let mut guard_input = GuardInput {
        messages: Vec::new(),
        tools: Some(Vec::new()),
    };
    for message in &filtered.messages {
        let raw = message
            .extra
            .get("content")
            .unwrap_or(&GuardrailValue::Null);
        let content = match normalize_content(raw)? {
            None => MessageContent::Text(String::new()),
            Some(MessageContent::Parts(parts)) if parts.is_empty() => {
                MessageContent::Text(String::new())
            }
            Some(content) => content,
        };
        guard_input.messages.push(Message {
            role: message.role.clone(),
            content: Some(content),
        });
    }
    Ok((guard_input, filtered.indices))
}

fn texts_input(texts: &[String]) -> (GuardInput, Vec<usize>) {
    let mut guard_input = GuardInput {
        messages: Vec::new(),
        tools: Some(Vec::new()),
    };
    for text in texts {
        guard_input.messages.push(Message {
            role: "user".to_owned(),
            content: Some(MessageContent::Text(text.clone())),
        });
    }
    (guard_input, (0..texts.len()).collect())
}

#[must_use]
pub fn build_guard_input_for_response(inputs: &GuardrailApiInputs) -> GuardInput {
    let texts = inputs.texts.as_deref().unwrap_or_default();
    GuardInput {
        messages: texts
            .iter()
            .map(|text| Message {
                role: "assistant".to_owned(),
                content: Some(MessageContent::Text(text.clone())),
            })
            .collect(),
        tools: Some(inputs.tools.clone().unwrap_or_default()),
    }
}

// LiteLLM's _extract_transformed_texts: a zero count takes nothing, not the whole list
#[must_use]
pub fn extract_transformed_texts(
    guard_output: &GuardInput,
    num_assistant_messages: usize,
) -> Vec<String> {
    if num_assistant_messages == 0 {
        return Vec::new();
    }
    let skip = guard_output
        .messages
        .len()
        .saturating_sub(num_assistant_messages);
    guard_output
        .messages
        .iter()
        .skip(skip)
        .map(extract_text_from_message)
        .collect()
}

mod tests;
