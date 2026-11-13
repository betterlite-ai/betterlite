use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{Fields, TEXT_PART_TYPES, normalize_content};
use crate::cisco_redaction::{set_serializable_field, to_raw};

type RedactionSource<'source> = (Option<&'source str>, Option<&'source [Fields]>);

#[derive(Clone, Copy)]
pub enum ListField {
    TextCompletion,
    ResponsesOutput,
}

#[must_use]
pub fn redact_list_field(
    response: Option<&mut Fields>,
    sanitized_text: Option<&str>,
    sanitized_messages: Option<&[Fields]>,
    kind: ListField,
) -> bool {
    let source = (sanitized_text, sanitized_messages);
    match kind {
        ListField::TextCompletion => redact_list_output(
            response,
            "choices",
            source,
            text_completion_replacement,
            |raw_choices, replacement| {
                map_items(raw_choices, |choice| {
                    redact_field_when_present(choice, "text", replacement)
                })
            },
        ),
        ListField::ResponsesOutput => redact_list_output(
            response,
            "output",
            source,
            responses_replacement_text,
            |raw_output, replacement_text| {
                map_items(raw_output, |item| {
                    let content_applied = redact_output_item_content(item, replacement_text);
                    let arguments_applied = clear_output_item_arguments(item);
                    content_applied || arguments_applied
                })
            },
        ),
    }
}

fn redact_list_output<Transform>(
    response: Option<&mut Fields>,
    field: &str,
    source: RedactionSource<'_>,
    compute_replacement: fn(RedactionSource<'_>) -> Option<String>,
    transform: Transform,
) -> bool
where
    Transform: FnOnce(&[Box<RawValue>], &str) -> (Vec<Box<RawValue>>, bool),
{
    let Some(present_response) = response else {
        return false;
    };
    let Json::List(raw_items) =
        Json::parse(present_response.get(field).map_or("null", |raw| raw.get()))
    else {
        return false;
    };
    let Some(text) = compute_replacement(source) else {
        return false;
    };
    let (items, applied) = transform(&raw_items, &text);
    set_serializable_field(present_response, field, &items);
    applied
}

pub(crate) fn redact_field_when_present<Value>(
    fields: &mut Fields,
    key: &str,
    replacement: &Value,
) -> bool
where
    Value: Serialize + ?Sized,
{
    let applied = Json::parse(fields.get(key).map_or("null", |raw| raw.get())).truthy();
    if applied {
        set_serializable_field(fields, key, replacement);
    }
    applied
}

fn explicit_replacement(sanitized_text: Option<&str>) -> Option<String> {
    sanitized_text
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn text_completion_replacement(
    (sanitized_text, sanitized_messages): RedactionSource<'_>,
) -> Option<String> {
    explicit_replacement(sanitized_text).or_else(|| {
        sanitized_messages?.iter().find_map(|message| {
            let raw = message.get("content")?;
            let text = normalize_content(&Json::parse(raw.get()));
            (!text.is_empty()).then_some(text)
        })
    })
}

fn responses_replacement_text(
    (sanitized_text, sanitized_messages): RedactionSource<'_>,
) -> Option<String> {
    if let Some(text) = explicit_replacement(sanitized_text) {
        return Some(text);
    }
    let joined = sanitized_messages?
        .iter()
        .map(|message| {
            normalize_content(&Json::parse(
                message.get("content").map_or("null", |raw| raw.get()),
            ))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = joined.trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub(crate) fn map_items<Mutate>(
    raw_items: &[Box<RawValue>],
    mut mutate: Mutate,
) -> (Vec<Box<RawValue>>, bool)
where
    Mutate: FnMut(&mut Fields) -> bool,
{
    let mut applied = false;
    let mut updated = Vec::with_capacity(raw_items.len());
    for raw_item in raw_items {
        let Json::Object(mut item) = Json::parse(raw_item.get()) else {
            updated.push(raw_item.clone());
            continue;
        };
        if mutate(&mut item) {
            applied = true;
        }
        updated.push(to_raw(&item).unwrap_or_else(|| raw_item.clone()));
    }
    (updated, applied)
}

fn redact_output_item_content(item: &mut Fields, replacement_text: &str) -> bool {
    let Json::List(parts) = Json::parse(item.get("content").map_or("null", |raw| raw.get())) else {
        return false;
    };
    let mut applied = false;
    let mut updated = Vec::with_capacity(parts.len());
    for raw_part in &parts {
        let Json::Object(mut part) = Json::parse(raw_part.get()) else {
            updated.push(raw_part.clone());
            continue;
        };
        if is_text_part(&part) {
            set_serializable_field(&mut part, "text", &replacement_text);
            applied = true;
        }
        updated.push(to_raw(&part).unwrap_or_else(|| raw_part.clone()));
    }
    if applied {
        set_serializable_field(item, "content", &updated);
    }
    applied
}

fn clear_output_item_arguments(item: &mut Fields) -> bool {
    match Json::parse(item.get("arguments").map_or("null", |raw| raw.get())) {
        Json::Text(text) if !text.is_empty() => {
            set_serializable_field(item, "arguments", "{}");
            true
        }
        Json::Text(_)
        | Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_) => false,
    }
}

fn is_text_part(part: &Fields) -> bool {
    match Json::parse(part.get("type").map_or("null", |raw| raw.get())) {
        Json::Text(kind) => TEXT_PART_TYPES.contains(&kind.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    }
}
