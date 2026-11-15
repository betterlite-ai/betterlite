use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::content_inspection::build_inspection_messages;

use super::fields::{non_empty_text, object_of};
use super::schema::tool_definition_text;
use super::tool_calls::tool_call_args;

#[must_use]
pub fn prompt_field_text(data: &BTreeMap<String, GuardrailValue>) -> Vec<&str> {
    match data.get("prompt") {
        Some(GuardrailValue::Text(prompt)) if !prompt.is_empty() => vec![prompt.as_str()],
        Some(GuardrailValue::Array(items)) => items
            .iter()
            .filter_map(|item| non_empty_text(Some(item)))
            .collect(),
        _ => Vec::new(),
    }
}

#[must_use]
pub fn input_text_parts(content: Option<&GuardrailValue>) -> Vec<&str> {
    let Some(GuardrailValue::Array(parts)) = content else {
        return Vec::new();
    };
    let mut texts = Vec::new();
    for part in parts {
        let Some(object) = object_of(part) else {
            continue;
        };
        if !matches!(object.get("type"), Some(GuardrailValue::Text(kind)) if kind == "input_text") {
            continue;
        }
        if let Some(text) = non_empty_text(object.get("text")) {
            texts.push(text);
        }
    }
    texts
}

#[must_use]
pub fn extract_prompt_text(data: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    let inspected = build_inspection_messages(data);
    let mut texts: Vec<&str> = inspected
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    texts.extend(prompt_field_text(data));
    if let Some(instructions) = non_empty_text(data.get("instructions")) {
        texts.push(instructions);
    }
    if let Some(GuardrailValue::Array(messages)) = data.get("messages") {
        for message in messages {
            texts.extend(tool_call_args(message));
        }
    }
    if let Some(GuardrailValue::Array(items)) = data.get("input") {
        for item in items {
            let Some(object) = object_of(item) else {
                continue;
            };
            if !object.contains_key("role") {
                continue;
            }
            texts.extend(tool_call_args(item));
            texts.extend(input_text_parts(object.get("content")));
        }
    }
    texts.extend(tool_definition_text(data));
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}
