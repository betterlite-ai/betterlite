use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

const OUTPUT_ITEM_TYPES: [&str; 2] = ["function_call_output", "custom_tool_call_output"];

pub(super) enum LiftedContent<'value> {
    Text(&'value str),
    Value(&'value GuardrailValue),
}

pub(super) struct LiftedMessage<'value> {
    pub(super) role: Option<&'value str>,
    pub(super) content: LiftedContent<'value>,
}

fn text_of(value: Option<&GuardrailValue>) -> Option<&str> {
    let Some(GuardrailValue::Text(text)) = value else {
        return None;
    };
    if text.is_empty() { None } else { Some(text) }
}

fn part_text(part: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(object) = part else {
        return None;
    };
    text_of(object.get("text"))
}

fn part_fragments(part: &GuardrailValue) -> Vec<&str> {
    if let GuardrailValue::Text(text) = part {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text]
        };
    }
    part_text(part).into_iter().collect()
}

pub(super) fn content_value_texts(content: &GuardrailValue) -> Vec<&str> {
    match content {
        GuardrailValue::Text(text) if !text.is_empty() => vec![text],
        GuardrailValue::Array(parts) => parts.iter().flat_map(part_fragments).collect(),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Object(_) => Vec::new(),
    }
}

pub(super) fn content_texts<'value>(content: &LiftedContent<'value>) -> Vec<&'value str> {
    match content {
        LiftedContent::Text(text) if !text.is_empty() => vec![text],
        LiftedContent::Text(_) => Vec::new(),
        LiftedContent::Value(value) => content_value_texts(value),
    }
}

fn lift_reasoning(object: &BTreeMap<String, GuardrailValue>) -> Vec<LiftedMessage<'_>> {
    let role = text_of(object.get("role")).unwrap_or("assistant");
    let mut messages = Vec::new();
    if let Some(content) = object.get("content") {
        messages.push(LiftedMessage {
            role: Some(role),
            content: LiftedContent::Value(content),
        });
    }
    if let Some(summary @ GuardrailValue::Array(_)) = object.get("summary") {
        messages.push(LiftedMessage {
            role: Some(role),
            content: LiftedContent::Value(summary),
        });
    }
    messages
}

fn lift_object_item<'value>(
    item: &'value GuardrailValue,
    object: &'value BTreeMap<String, GuardrailValue>,
) -> Vec<LiftedMessage<'value>> {
    if let Some(text) = part_text(item) {
        return vec![LiftedMessage {
            role: Some(text_of(object.get("role")).unwrap_or("user")),
            content: LiftedContent::Text(text),
        }];
    }
    if text_of(object.get("type")) == Some("reasoning") {
        return lift_reasoning(object);
    }
    if let Some(content) = object.get("content") {
        return vec![LiftedMessage {
            role: Some(text_of(object.get("role")).unwrap_or("user")),
            content: LiftedContent::Value(content),
        }];
    }
    let is_output =
        text_of(object.get("type")).is_some_and(|kind| OUTPUT_ITEM_TYPES.contains(&kind));
    match object.get("output") {
        Some(output) if is_output => vec![LiftedMessage {
            role: Some(text_of(object.get("role")).unwrap_or("tool")),
            content: LiftedContent::Value(output),
        }],
        _ => Vec::new(),
    }
}

fn lift_input(input: Option<&GuardrailValue>) -> Vec<LiftedMessage<'_>> {
    let Some(value) = input else {
        return Vec::new();
    };
    if let GuardrailValue::Text(text) = value {
        return vec![LiftedMessage {
            role: Some("user"),
            content: LiftedContent::Text(text),
        }];
    }
    let GuardrailValue::Array(items) = value else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    for item in items {
        match item {
            GuardrailValue::Text(text) => messages.push(LiftedMessage {
                role: Some("user"),
                content: LiftedContent::Text(text),
            }),
            GuardrailValue::Object(object) => messages.extend(lift_object_item(item, object)),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_) => {}
        }
    }
    messages
}

pub(super) fn inspected_messages(
    data: &BTreeMap<String, GuardrailValue>,
) -> Vec<LiftedMessage<'_>> {
    let mut messages = Vec::new();
    if let Some(GuardrailValue::Array(entries)) = data.get("messages") {
        for entry in entries {
            if let GuardrailValue::Object(object) = entry
                && let Some(content) = object.get("content")
            {
                messages.push(LiftedMessage {
                    role: text_of(object.get("role")),
                    content: LiftedContent::Value(content),
                });
            }
        }
    }
    messages.extend(lift_input(data.get("input")));
    messages
}
