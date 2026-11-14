use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fields::{get_field, non_empty_text, object_of};
use super::tool_calls::tool_call_args;

pub enum ResponseSource<'response> {
    Body(&'response BTreeMap<String, GuardrailValue>),
    OutputText(&'response str),
}

fn response_array<'response>(
    response: &'response BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<&'response Vec<GuardrailValue>> {
    match response.get(key) {
        Some(GuardrailValue::Array(items)) => Some(items),
        _ => None,
    }
}

fn joined(parts: &[&str], separator: &str) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(separator))
    }
}

#[must_use]
pub fn extract_response_text(response: &ResponseSource<'_>) -> Option<String> {
    match response {
        ResponseSource::Body(body) => {
            chat_completion_text(body).or_else(|| responses_api_text(body))
        }
        ResponseSource::OutputText(text) => {
            if text.is_empty() {
                None
            } else {
                Some(String::from(*text))
            }
        }
    }
}

#[must_use]
pub fn chat_completion_text(response: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    let choices = response_array(response, "choices")?;
    let mut parts: Vec<&str> = Vec::new();
    for choice in choices {
        if let Some(message) = get_field(Some(choice), "message")
            && let Some(object) = object_of(message)
        {
            if let Some(content) = non_empty_text(object.get("content")) {
                parts.push(content);
            }
            parts.extend(tool_call_args(message));
        }
        if let Some(text) = non_empty_text(get_field(Some(choice), "text")) {
            parts.push(text);
        }
    }
    joined(&parts, "\n")
}

fn function_call_arguments(item: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    if !matches!(item.get("type"), Some(GuardrailValue::Text(kind)) if kind == "function_call") {
        return None;
    }
    non_empty_text(item.get("arguments"))
}

fn output_message_text(item: &BTreeMap<String, GuardrailValue>) -> Vec<&str> {
    if !matches!(item.get("type"), Some(GuardrailValue::Text(kind)) if kind == "message") {
        return Vec::new();
    }
    let Some(GuardrailValue::Array(content)) = item.get("content") else {
        return Vec::new();
    };
    let mut texts = Vec::new();
    for part in content {
        let Some(object) = object_of(part) else {
            continue;
        };
        let is_text_part = matches!(
            object.get("type"),
            Some(GuardrailValue::Text(kind)) if kind == "output_text" || kind == "text"
        );
        if !is_text_part {
            continue;
        }
        if let Some(text) = non_empty_text(object.get("text")) {
            texts.push(text);
        }
    }
    texts
}

fn output_item_text(item: &GuardrailValue) -> Vec<&str> {
    let Some(object) = object_of(item) else {
        return Vec::new();
    };
    if let Some(arguments) = function_call_arguments(object) {
        return vec![arguments];
    }
    output_message_text(object)
}

#[must_use]
pub fn responses_api_text(response: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    let output = response_array(response, "output")?;
    let texts: Vec<&str> = output.iter().flat_map(output_item_text).collect();
    joined(&texts, "")
}
