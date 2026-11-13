use std::collections::BTreeMap;

use crate::otel::model::utils::coercion::OtelModelValue;

pub mod tools;

pub fn transform_messages(messages: &OtelModelValue) -> Vec<OtelModelValue> {
    match messages {
        OtelModelValue::String(instructions) => {
            vec![text_message("system", vec![content_part(instructions)])]
        }
        OtelModelValue::Array(items) => items.iter().filter_map(transformed_message).collect(),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::Object(_) => Vec::new(),
    }
}

pub fn transform_choices(choices: &[OtelModelValue]) -> Vec<OtelModelValue> {
    choices
        .iter()
        .filter_map(transformed_choice_message)
        .collect()
}

pub fn transform_responses_output(output: &[OtelModelValue]) -> Vec<OtelModelValue> {
    output
        .iter()
        .filter_map(transformed_response_item)
        .collect()
}

fn transformed_choice_message(choice: &OtelModelValue) -> Option<OtelModelValue> {
    let fields = choice.as_object()?;
    let message = fields
        .get("message")
        .and_then(OtelModelValue::as_object)
        .cloned()
        .unwrap_or_default();
    let mut messages = transform_messages(&OtelModelValue::Array(vec![OtelModelValue::Object(
        message,
    )]));
    let OtelModelValue::Object(mut parts_message) = messages.pop()? else {
        return None;
    };
    let reason = fields
        .get("finish_reason")
        .filter(|reason| !matches!(reason, OtelModelValue::Null(())));
    if let Some(text) = reason {
        parts_message.insert("finish_reason".to_owned(), text.clone());
    }
    Some(OtelModelValue::Object(parts_message))
}

fn transformed_response_item(item: &OtelModelValue) -> Option<OtelModelValue> {
    let fields = item.as_object()?;
    match fields.get("type").and_then(OtelModelValue::as_str) {
        Some("message") => response_message(fields),
        Some("function_call") => Some(function_call_message(fields)),
        Some(_) | None => None,
    }
}

fn response_message(fields: &BTreeMap<String, OtelModelValue>) -> Option<OtelModelValue> {
    let role = fields
        .get("role")
        .and_then(OtelModelValue::as_str)
        .unwrap_or("assistant");
    let parts = output_text_parts(fields.get("content"));
    (!parts.is_empty()).then(|| text_message(role, parts))
}

fn output_text_parts(content: Option<&OtelModelValue>) -> Vec<OtelModelValue> {
    let Some(items) = content.and_then(OtelModelValue::as_array) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for raw_part in items {
        let Some(part) = raw_part.as_object() else {
            continue;
        };
        if part.get("type").and_then(OtelModelValue::as_str) != Some("output_text") {
            continue;
        }
        let text = part
            .get("text")
            .and_then(OtelModelValue::as_str)
            .unwrap_or_default();
        if !text.is_empty() {
            parts.push(content_part(text));
        }
    }
    parts
}

fn function_call_message(fields: &BTreeMap<String, OtelModelValue>) -> OtelModelValue {
    let mut part = BTreeMap::from([
        ("arguments".to_owned(), field_value(fields, "arguments")),
        ("name".to_owned(), field_value(fields, "name")),
        (
            "type".to_owned(),
            OtelModelValue::String("tool_call".to_owned()),
        ),
    ]);
    let identifier = fields
        .get("call_id")
        .and_then(OtelModelValue::as_str)
        .filter(|call_id| !call_id.is_empty());
    if let Some(call_id) = identifier {
        part.insert("id".to_owned(), OtelModelValue::String(call_id.to_owned()));
    }
    text_message("assistant", vec![OtelModelValue::Object(part)])
}

fn transformed_message(message: &OtelModelValue) -> Option<OtelModelValue> {
    let fields = message.as_object()?;
    let role = fields
        .get("role")
        .and_then(OtelModelValue::as_str)
        .unwrap_or("user");
    let mut parts = Vec::new();
    match fields.get("content") {
        None => parts.push(content_part("")),
        Some(OtelModelValue::String(content)) => parts.push(content_part(content)),
        Some(OtelModelValue::Array(items)) => {
            for part in items {
                match part {
                    OtelModelValue::String(text) => parts.push(content_part(text)),
                    OtelModelValue::Object(_) => parts.push(part.clone()),
                    other @ (OtelModelValue::Null(())
                    | OtelModelValue::Bool(_)
                    | OtelModelValue::Number(_)
                    | OtelModelValue::Array(_)) => {
                        let text = serde_json::to_string(other).unwrap_or_default();
                        parts.push(content_part(&text));
                    }
                }
            }
        }
        Some(_) => {}
    }
    let mut encoded = BTreeMap::from([
        ("parts".to_owned(), OtelModelValue::Array(parts)),
        ("role".to_owned(), OtelModelValue::String(role.to_owned())),
    ]);
    for key in ["id", "tool_calls", "tool_call_id"] {
        if let Some(value) = fields.get(key) {
            encoded.insert(key.to_owned(), value.clone());
        }
    }
    Some(OtelModelValue::Object(encoded))
}

fn field_value(fields: &BTreeMap<String, OtelModelValue>, key: &str) -> OtelModelValue {
    fields
        .get(key)
        .cloned()
        .unwrap_or_else(|| OtelModelValue::String(String::new()))
}

fn content_part(content: &str) -> OtelModelValue {
    OtelModelValue::Object(BTreeMap::from([
        (
            "content".to_owned(),
            OtelModelValue::String(content.to_owned()),
        ),
        ("type".to_owned(), OtelModelValue::String("text".to_owned())),
    ]))
}

fn text_message(role: &str, parts: Vec<OtelModelValue>) -> OtelModelValue {
    OtelModelValue::Object(BTreeMap::from([
        ("parts".to_owned(), OtelModelValue::Array(parts)),
        ("role".to_owned(), OtelModelValue::String(role.to_owned())),
    ]))
}

#[path = "transforms/otel_transforms/mod.rs"]
mod otel_transforms;
