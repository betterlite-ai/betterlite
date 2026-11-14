use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::crowdstrike_aidr::models::{GuardInput, Message, MessageContent, content_part_value};
use crate::crowdstrike_normalization::{extract_text_from_content, extract_text_from_message};
use crate::generic_guardrail_contracts::GuardrailMessage;

// The two per-guardrail skip switches LiteLLM reads through the effective_skip helpers
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkipFlags {
    pub system: bool,
    pub tool: bool,
}

// LiteLLM's _merge_request_transforms: returned text at a sent position replaces that original text
#[must_use]
pub fn merge_request_transforms(
    guard_output: &GuardInput,
    structured_messages: Option<&[GuardrailMessage]>,
    texts: &[String],
    sent_indices: &[usize],
) -> Vec<String> {
    let returned: Vec<String> = guard_output
        .messages
        .iter()
        .map(extract_text_from_message)
        .collect();
    let original: Vec<String> = match structured_messages.filter(|messages| !messages.is_empty()) {
        Some(messages) => messages
            .iter()
            .map(|message| extract_text_from_content(content_of(message)))
            .collect(),
        None => texts.to_vec(),
    };
    let mut replacements = BTreeMap::new();
    for (position, index) in sent_indices.iter().enumerate() {
        if let Some(returned_text) = returned.get(position)
            && original.get(*index).is_some()
        {
            replacements.insert(*index, returned_text.clone());
        }
    }
    original
        .iter()
        .enumerate()
        .map(|(index, text)| {
            replacements
                .get(&index)
                .cloned()
                .unwrap_or_else(|| text.clone())
        })
        .collect()
}

fn content_of(message: &GuardrailMessage) -> &GuardrailValue {
    message
        .extra
        .get("content")
        .unwrap_or(&GuardrailValue::Null)
}

// LiteLLM's _apply_message_redaction: a string content takes the redacted text, a text list takes the redacted list
#[must_use]
pub fn apply_message_redaction(
    original: &GuardrailMessage,
    redacted: &Message,
) -> GuardrailMessage {
    let Some(content) = original.extra.get("content") else {
        return original.clone();
    };
    if let GuardrailValue::Text(_) = content {
        return with_content(
            original,
            GuardrailValue::Text(extract_text_from_message(redacted)),
        );
    }
    if let GuardrailValue::Array(_) = content
        && !extract_text_from_content(content).is_empty()
    {
        return with_content(original, redacted_content(redacted));
    }
    original.clone()
}

fn with_content(original: &GuardrailMessage, content: GuardrailValue) -> GuardrailMessage {
    let mut extra = original.extra.clone();
    extra.insert("content".to_owned(), content);
    GuardrailMessage {
        role: original.role.clone(),
        extra,
    }
}

fn redacted_content(redacted: &Message) -> GuardrailValue {
    match &redacted.content {
        None => GuardrailValue::Null,
        Some(MessageContent::Text(text)) => GuardrailValue::Text(text.clone()),
        Some(MessageContent::Parts(parts)) => {
            GuardrailValue::Array(parts.iter().map(content_part_value).collect())
        }
    }
}

// LiteLLM's _redacted_messages, with value equality standing in for the id() map
#[must_use]
pub fn redacted_messages(
    processed_messages: &[GuardrailMessage],
    guard_output: &GuardInput,
    sent_indices: &[usize],
    full_messages: &[GuardrailMessage],
) -> Option<Vec<GuardrailMessage>> {
    let mut repositioned = Vec::new();
    for (position, index) in sent_indices.iter().enumerate() {
        if position >= guard_output.messages.len() {
            continue;
        }
        let redacted = apply_message_redaction(
            processed_messages.get(*index)?,
            guard_output.messages.get(position)?,
        );
        let original = processed_messages.get(*index)?;
        let found = full_messages
            .iter()
            .position(|message| message == original)?;
        repositioned.push((found, redacted));
    }
    let mut result = full_messages.to_vec();
    for (position, redacted) in repositioned {
        if let Some(slot) = result.get_mut(position) {
            *slot = redacted;
        }
    }
    Some(result)
}

// LiteLLM's _writeback_messages: skipping enabled means the request's own message list is the target
#[must_use]
pub fn writeback_messages(
    structured_messages: &[GuardrailMessage],
    guard_output: &GuardInput,
    sent_indices: &[usize],
    request_data: &BTreeMap<String, GuardrailValue>,
    skip: SkipFlags,
) -> Option<Vec<GuardrailMessage>> {
    let full = if skip.system || skip.tool {
        request_messages(request_data).unwrap_or_else(|| structured_messages.to_vec())
    } else {
        structured_messages.to_vec()
    };
    redacted_messages(structured_messages, guard_output, sent_indices, &full)
}

// a request message list that is not several message objects keeps the structured list instead
fn request_messages(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<Vec<GuardrailMessage>> {
    let GuardrailValue::Array(items) = request_data.get("messages")? else {
        return None;
    };
    let encoded = serde_json::to_value(GuardrailValue::Array(items.clone())).ok()?;
    serde_json::from_value(encoded).ok()
}

mod tests;
