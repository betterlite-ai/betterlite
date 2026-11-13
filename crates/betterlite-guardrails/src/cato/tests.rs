#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

mod actions;
mod constructor;
mod inspection;
mod redaction;

pub(super) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(super) fn integer(value: i64) -> GuardrailValue {
    GuardrailValue::SignedInteger(value)
}

pub(super) fn array_of(data: &BTreeMap<String, GuardrailValue>, key: &str) -> Vec<GuardrailValue> {
    match data.get(key) {
        Some(GuardrailValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

pub(super) fn list(items: &[GuardrailValue]) -> GuardrailValue {
    GuardrailValue::Array(items.to_vec())
}

pub(super) fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

pub(super) fn body(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

// the text of one field per message, or an empty string when a message carries none
pub(super) fn field_text(messages: &[GuardrailValue], field: &str) -> Vec<String> {
    messages
        .iter()
        .map(|message| match message {
            GuardrailValue::Object(object) => match object.get(field) {
                Some(GuardrailValue::Text(text)) => text.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        })
        .collect()
}

pub(super) fn message_pairs(messages: &[GuardrailValue]) -> Vec<(String, String)> {
    field_text(messages, "role")
        .into_iter()
        .zip(field_text(messages, "content"))
        .collect()
}
