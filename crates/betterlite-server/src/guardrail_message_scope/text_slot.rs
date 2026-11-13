use std::slice::Iter;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

mod tests;

fn part_text(part: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(fields) = part else {
        return None;
    };
    let Some(GuardrailValue::Text(text)) = fields.get("text") else {
        return None;
    };
    Some(text)
}

// A slot is a string content or one list part carrying a string text; the part's type is not looked at.
#[must_use]
pub fn message_slot_texts(message: &Object) -> Vec<&str> {
    match message.get("content") {
        Some(GuardrailValue::Text(text)) => vec![text],
        Some(GuardrailValue::Array(parts)) => parts.iter().filter_map(part_text).collect(),
        _ => Vec::new(),
    }
}

fn part_with_next_text(part: &GuardrailValue, texts: &mut Iter<'_, String>) -> GuardrailValue {
    let GuardrailValue::Object(fields) = part else {
        return part.clone();
    };
    if part_text(part).is_none() {
        return part.clone();
    }
    let Some(text) = texts.next() else {
        return part.clone();
    };
    let mut rewritten = fields.clone();
    rewritten.insert("text".to_owned(), GuardrailValue::Text(text.clone()));
    GuardrailValue::Object(rewritten)
}

// None unless the counts line up exactly, so a rewrite never lands on the wrong slot.
#[must_use]
pub fn message_with_slot_texts(message: &Object, texts: &[String]) -> Option<Object> {
    if message_slot_texts(message).len() != texts.len() {
        return None;
    }
    let mut rewritten = message.clone();
    match message.get("content") {
        Some(GuardrailValue::Text(_)) => {
            let text = texts.first()?;
            rewritten.insert("content".to_owned(), GuardrailValue::Text(text.clone()));
        }
        Some(GuardrailValue::Array(parts)) => {
            let mut remaining = texts.iter();
            let rewritten_parts = parts
                .iter()
                .map(|part| part_with_next_text(part, &mut remaining))
                .collect();
            rewritten.insert("content".to_owned(), GuardrailValue::Array(rewritten_parts));
        }
        _ => {}
    }
    Some(rewritten)
}
