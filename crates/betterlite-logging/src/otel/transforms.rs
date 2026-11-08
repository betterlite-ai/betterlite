use std::collections::BTreeMap;

use crate::otel::model::utils::coercion::OtelModelValue;

pub mod tools;

pub fn transform_messages(messages: &OtelModelValue) -> Vec<OtelModelValue> {
    if let Some(instructions) = messages.as_str() {
        return vec![text_message("system", vec![content_part(instructions)])];
    }
    let Some(items) = messages.as_array() else {
        return Vec::new();
    };
    items.iter().filter_map(transformed_message).collect()
}

pub fn transform_choices(choices: &[OtelModelValue]) -> Vec<OtelModelValue> {
    let mut transformed = Vec::new();
    for choice in choices {
        let Some(fields) = choice.as_object() else {
            continue;
        };
        let message = fields
            .get("message")
            .and_then(OtelModelValue::as_object)
            .cloned()
            .unwrap_or_default();
        let mut messages =
            transform_messages(&OtelModelValue::Array(vec![OtelModelValue::Object(
                message,
            )]));
        let Some(OtelModelValue::Object(mut parts_message)) = messages.pop() else {
            continue;
        };
        if let Some(reason) = fields
            .get("finish_reason")
            .filter(|reason| !matches!(reason, OtelModelValue::Null(())))
        {
            parts_message.insert("finish_reason".to_owned(), reason.clone());
        }
        transformed.push(OtelModelValue::Object(parts_message));
    }
    transformed
}

pub fn transform_responses_output(output: &[OtelModelValue]) -> Vec<OtelModelValue> {
    let mut transformed = Vec::new();
    for item in output {
        let Some(fields) = item.as_object() else {
            continue;
        };
        match fields.get("type").and_then(OtelModelValue::as_str) {
            Some("message") => {
                let role = fields
                    .get("role")
                    .and_then(OtelModelValue::as_str)
                    .unwrap_or("assistant");
                let parts = output_text_parts(fields.get("content"));
                if !parts.is_empty() {
                    transformed.push(text_message(role, parts));
                }
            }
            Some("function_call") => transformed.push(function_call_message(fields)),
            Some(_) | None => {}
        }
    }
    transformed
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
