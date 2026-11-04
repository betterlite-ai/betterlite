use serde_json::{Map, Value};

use super::debug::{display_text_value, format_diagnostic_value, text_block};
use super::image_file::input_file;
use super::{ChatContent, Projection};

pub fn text_by_role(content: &Value, role: &str) -> Value {
    match content {
        Value::Null => Value::Array(vec![text_block("", role).0]),
        Value::String(text) => Value::Array(vec![text_block(text, role).0]),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter_map(|item| text_item(item, role))
                .collect(),
        ),
        Value::Object(fields) => Value::Array(vec![
            text_block(
                &display_text_value(fields.get("text").unwrap_or(content)),
                role,
            )
            .0,
        ]),
        _ => Value::Array(vec![text_block(&display_text_value(content), role).0]),
    }
}

fn text_item(item: &Value, role: &str) -> Option<Value> {
    match item {
        Value::String(text) => Some(text_block(text, role).0),
        Value::Object(fields) if fields.get("type").and_then(Value::as_str) == Some("text") => {
            Some(
                text_block(
                    fields.get("text").and_then(Value::as_str).unwrap_or(""),
                    role,
                )
                .0,
            )
        }
        Value::Object(fields) => fields
            .get("text")
            .and_then(Value::as_str)
            .map(|text| text_block(text, role).0),
        _ => None,
    }
}

pub fn content_passthrough(items: &[Value], role: &str) -> Projection {
    let mut result = Vec::new();
    let mut result_reprs = Vec::new();
    let mut diagnostics = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("tool_reference") {
            diagnostics.push(
                "Chat provider:   tool_reference has no responses API equivalent; skipped"
                    .to_owned(),
            );
            continue;
        }
        if let Some((value, repr, diagnostic)) = project_item(item, role) {
            if let Some(message) = diagnostic {
                diagnostics.push(message);
            }
            result.push(value);
            result_reprs.push(repr);
        }
    }
    diagnostics.push(format!(
        "Chat provider: Final converted content: [{}]",
        result_reprs.join(", ")
    ));
    Projection {
        value: Value::Array(result),
        diagnostics,
    }
}

fn project_item(item: &Value, role: &str) -> Option<(Value, String, Option<String>)> {
    match item {
        Value::String(text) => {
            let (value, repr) = text_block(text, role);
            Some((
                value,
                repr.clone(),
                Some(format!("Chat provider:   -> {repr}")),
            ))
        }
        Value::Object(fields) => Some(project_object(item, fields, role)),
        _ => None,
    }
}

fn project_object(
    item: &Value,
    fields: &Map<String, Value>,
    role: &str,
) -> (Value, String, Option<String>) {
    let Some(kind) = fields.get("type").and_then(Value::as_str) else {
        return passthrough(item);
    };
    match kind {
        "text" => {
            let text = fields.get("text").and_then(Value::as_str).unwrap_or("");
            let (value, repr) = text_block(text, role);
            (
                value,
                repr.clone(),
                Some(format!("Chat provider:   text -> {repr}")),
            )
        }
        "image" => image_item(fields),
        "file" => file_item(fields),
        "tool_reference" => passthrough(item),
        "input_text"
        | "input_image"
        | "output_text"
        | "refusal"
        | "input_file"
        | "computer_screenshot"
        | "summary_text" => passthrough(item),
        _ => unknown_item(item, fields, kind, role),
    }
}

fn passthrough(item: &Value) -> (Value, String, Option<String>) {
    let repr = format_diagnostic_value(item);
    (
        item.clone(),
        repr.clone(),
        Some(format!("Chat provider:   passthrough -> {repr}")),
    )
}

fn image_item(fields: &Map<String, Value>) -> (Value, String, Option<String>) {
    let mut image = Map::new();
    image.insert("type".to_owned(), Value::String("input_image".to_owned()));
    image.extend(fields.clone());
    let value = Value::Object(image);
    let repr = format_diagnostic_value(&value);
    (
        value,
        repr.clone(),
        Some(format!("Chat provider:   image -> {repr}")),
    )
}

fn file_item(fields: &Map<String, Value>) -> (Value, String, Option<String>) {
    let value = input_file(fields.get("file"));
    let repr = format_diagnostic_value(&value);
    (
        value,
        repr.clone(),
        Some(format!("Chat provider:   file -> {repr}")),
    )
}

fn unknown_item(
    item: &Value,
    fields: &Map<String, Value>,
    kind: &str,
    role: &str,
) -> (Value, String, Option<String>) {
    let unknown = ChatContent::Unknown(item.clone());
    let ChatContent::Unknown(value) = unknown else {
        return passthrough(item);
    };
    let text = fields
        .get("text")
        .map_or_else(|| display_text_value(&value), display_text_value);
    let (converted, repr) = text_block(&text, role);
    (
        converted,
        repr.clone(),
        Some(format!("Chat provider:   unknown({kind}) -> {repr}")),
    )
}
