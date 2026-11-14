use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use indexmap::IndexMap;

pub(crate) const CHUNK_OBJECT: &str = "chat.completion.chunk";

const NO_CHOICES: &[GuardrailValue] = &[];

#[must_use]
pub fn chunk_choices(item: &GuardrailValue) -> &[GuardrailValue] {
    let GuardrailValue::Object(fields) = item else {
        return NO_CHOICES;
    };
    let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
        return NO_CHOICES;
    };
    choices
}

fn object_field<'item>(value: &'item GuardrailValue, key: &str) -> Option<&'item GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    fields.get(key)
}

fn delta_field<'item>(choice: &'item GuardrailValue, key: &str) -> Option<&'item GuardrailValue> {
    let GuardrailValue::Object(fields) = choice else {
        return None;
    };
    let Some(GuardrailValue::Object(delta)) = fields.get("delta") else {
        return None;
    };
    delta.get(key)
}

// LiteLLM reads index with `getattr(choice, "index", 0) or 0`, so a null index counts as 0
fn choice_index(choice: &GuardrailValue) -> i64 {
    match object_field(choice, "index") {
        Some(GuardrailValue::SignedInteger(index)) => *index,
        Some(GuardrailValue::UnsignedInteger(index)) => i64::try_from(*index).unwrap_or(i64::MAX),
        _ => 0,
    }
}

// StreamingChoices maps a non-empty finish_reason and turns an empty one into None
fn finish_reason(choice: &GuardrailValue) -> Option<&str> {
    match object_field(choice, "finish_reason") {
        Some(GuardrailValue::Text(reason)) if !reason.is_empty() => Some(reason),
        _ => None,
    }
}

#[must_use]
pub fn chunk_has_tool_calls(item: &GuardrailValue) -> bool {
    chunk_choices(item).iter().any(|choice| {
        matches!(delta_field(choice, "tool_calls"), Some(GuardrailValue::Array(calls)) if !calls.is_empty())
    })
}

#[must_use]
pub fn chunk_carries_text(item: &GuardrailValue) -> bool {
    chunk_choices(item).iter().any(|choice| {
        matches!(delta_field(choice, "content"), Some(GuardrailValue::Text(text)) if !text.is_empty())
    })
}

#[must_use]
pub fn chunk_has_finish_reason(item: &GuardrailValue) -> bool {
    chunk_choices(item)
        .iter()
        .any(|choice| finish_reason(choice).is_some())
}

#[must_use]
pub fn is_trailing_metadata_chunk(item: &GuardrailValue) -> bool {
    if chunk_choices(item).is_empty() {
        return true;
    }
    let carries_usage = matches!(object_field(item, "usage"), Some(usage) if !matches!(usage, GuardrailValue::Null));
    carries_usage && !chunk_carries_text(item) && !chunk_has_finish_reason(item)
}

#[must_use]
pub fn trailing_metadata_chunks(items: &[GuardrailValue]) -> Vec<&GuardrailValue> {
    items
        .iter()
        .filter(|item| is_trailing_metadata_chunk(item))
        .collect()
}

pub fn record_finish_reasons(
    item: &GuardrailValue,
    finish_reason_per_choice: &mut IndexMap<i64, Option<String>>,
) {
    for choice in chunk_choices(item) {
        let Some(reason) = finish_reason(choice) else {
            continue;
        };
        finish_reason_per_choice.insert(
            choice_index(choice),
            Some(map_finish_reason(reason).reason.as_str().to_owned()),
        );
    }
}

#[must_use]
pub fn held_choices(held_chars_per_choice: &IndexMap<i64, i64>) -> BTreeSet<i64> {
    let mut held = BTreeSet::new();
    for (index, count) in held_chars_per_choice {
        if *count > 0 {
            held.insert(*index);
        }
    }
    held
}

#[must_use]
pub fn tool_call_passthrough_chunk(
    item: &GuardrailValue,
    mut finish_reason_per_choice: Option<&mut IndexMap<i64, Option<String>>>,
    held_choices: &BTreeSet<i64>,
) -> GuardrailValue {
    let mut choices = Vec::new();
    for choice in chunk_choices(item) {
        let index = choice_index(choice);
        let carries_text = matches!(
            delta_field(choice, "content"),
            Some(GuardrailValue::Text(text)) if !text.is_empty()
        );
        let text_pending = carries_text || held_choices.contains(&index);
        let original = finish_reason(choice);
        let passthrough = if let (true, Some(reason), Some(map)) = (
            text_pending,
            original,
            finish_reason_per_choice.as_deref_mut(),
        ) {
            map.insert(
                index,
                Some(map_finish_reason(reason).reason.as_str().to_owned()),
            );
            None
        } else {
            original.map(|reason| map_finish_reason(reason).reason.as_str().to_owned())
        };
        let role = delta_field(choice, "role")
            .cloned()
            .unwrap_or(GuardrailValue::Null);
        let tool_calls = delta_field(choice, "tool_calls")
            .cloned()
            .unwrap_or(GuardrailValue::Null);
        choices.push(stream_choice(
            index,
            GuardrailValue::Null,
            role,
            tool_calls,
            passthrough,
        ));
    }
    response_chunk(item, choices)
}

pub(crate) fn stream_choice(
    index: i64,
    content: GuardrailValue,
    role: GuardrailValue,
    tool_calls: GuardrailValue,
    finish_reason: Option<String>,
) -> GuardrailValue {
    let mut delta = BTreeMap::new();
    delta.insert("content".to_owned(), content);
    delta.insert("role".to_owned(), role);
    delta.insert("tool_calls".to_owned(), tool_calls);
    let mut choice = BTreeMap::new();
    choice.insert("index".to_owned(), GuardrailValue::SignedInteger(index));
    choice.insert("delta".to_owned(), GuardrailValue::Object(delta));
    choice.insert(
        "finish_reason".to_owned(),
        finish_reason.map_or(GuardrailValue::Null, GuardrailValue::Text),
    );
    GuardrailValue::Object(choice)
}

pub(crate) fn response_chunk(
    reference: &GuardrailValue,
    choices: Vec<GuardrailValue>,
) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    for key in ["id", "created", "model"] {
        let value = object_field(reference, key)
            .cloned()
            .unwrap_or(GuardrailValue::Null);
        fields.insert(key.to_owned(), value);
    }
    fields.insert(
        "object".to_owned(),
        GuardrailValue::Text(CHUNK_OBJECT.to_owned()),
    );
    fields.insert("choices".to_owned(), GuardrailValue::Array(choices));
    GuardrailValue::Object(fields)
}
