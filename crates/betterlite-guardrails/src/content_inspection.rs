use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

mod lifting;
mod tests;

use lifting::{content_texts, content_value_texts, inspected_messages};

pub const TEXT_CONTENT_CALL_TYPES: [&str; 3] = ["completion", "acompletion", "aresponses"];
pub const NON_CONVERSATIONAL_CALL_TYPES: [&str; 2] = ["embedding", "aembedding"];
pub const TEXT_PART_TYPES: [&str; 5] = [
    "text",
    "input_text",
    "output_text",
    "summary_text",
    "reasoning_text",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectionMessage {
    pub role: String,
    pub content: String,
}

#[must_use]
pub fn iter_message_text(data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    let mut texts = Vec::new();
    for message in inspected_messages(data) {
        texts.extend(
            content_texts(&message.content)
                .into_iter()
                .map(str::to_owned),
        );
    }
    texts
}

#[must_use]
pub fn build_inspection_messages(
    data: &BTreeMap<String, GuardrailValue>,
) -> Vec<InspectionMessage> {
    let mut inspected = Vec::new();
    for message in inspected_messages(data) {
        let content = content_texts(&message.content).join("\n");
        if content.is_empty() {
            continue;
        }
        inspected.push(InspectionMessage {
            role: message.role.unwrap_or("user").to_owned(),
            content,
        });
    }
    inspected
}

#[must_use]
pub fn is_string_batch_input(data: &BTreeMap<String, GuardrailValue>) -> bool {
    if data.contains_key("messages") {
        return false;
    }
    let Some(GuardrailValue::Array(items)) = data.get("input") else {
        return false;
    };
    !items.is_empty()
        && items
            .iter()
            .all(|item| matches!(item, GuardrailValue::Text(_)))
}

#[must_use]
pub fn has_non_string_content(data: &BTreeMap<String, GuardrailValue>) -> bool {
    if let Some(GuardrailValue::Array(messages)) = data.get("messages") {
        for message in messages {
            if let GuardrailValue::Object(object) = message
                && !matches!(
                    object.get("content"),
                    None | Some(GuardrailValue::Null | GuardrailValue::Text(_))
                )
            {
                return true;
            }
        }
    }
    !matches!(
        data.get("input"),
        None | Some(GuardrailValue::Null | GuardrailValue::Text(_))
    )
}

fn apply_string_batch(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted_messages: &[GuardrailValue],
) -> bool {
    let Some(GuardrailValue::Array(batch)) = data.get_mut("input") else {
        return false;
    };
    let inspected = batch
        .iter()
        .filter(|item| matches!(item, GuardrailValue::Text(text) if !text.is_empty()))
        .count();
    if redacted_messages.len() != inspected {
        return false;
    }
    let mut replacements = Vec::with_capacity(redacted_messages.len());
    for message in redacted_messages {
        let GuardrailValue::Object(object) = message else {
            return false;
        };
        let Some(content) = object.get("content") else {
            return false;
        };
        if matches!(content, GuardrailValue::Null) {
            return false;
        }
        replacements.push(content_value_texts(content).join("\n"));
    }
    // every entry is checked before the first write, since a partial rewrite forwards unredacted text
    let mut remaining = replacements.into_iter();
    for item in batch.iter_mut() {
        if let GuardrailValue::Text(text) = item
            && !text.is_empty()
        {
            let Some(replacement) = remaining.next() else {
                break;
            };
            *text = replacement;
        }
    }
    true
}

fn rewrite_input_string(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted_messages: &[GuardrailValue],
) {
    let mut texts = Vec::new();
    for message in redacted_messages {
        if let GuardrailValue::Object(object) = message
            && let Some(content) = object.get("content")
        {
            texts.extend(content_value_texts(content));
        }
    }
    data.insert("input".to_owned(), GuardrailValue::Text(texts.join("\n")));
}

#[must_use]
pub fn apply_redacted_messages_back(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted_messages: &[GuardrailValue],
) -> bool {
    if is_string_batch_input(data) {
        return apply_string_batch(data, redacted_messages);
    }
    if data.contains_key("messages") {
        data.insert(
            "messages".to_owned(),
            GuardrailValue::Array(redacted_messages.to_vec()),
        );
    }
    if let Some(GuardrailValue::Text(_)) = data.get("input") {
        rewrite_input_string(data, redacted_messages);
    }
    true
}
