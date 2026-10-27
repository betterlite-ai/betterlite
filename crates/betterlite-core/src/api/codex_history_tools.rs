#![expect(
    clippy::default_numeric_fallback,
    clippy::derive_partial_eq_without_eq,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::must_use_candidate,
    clippy::shadow_reuse,
    clippy::shadow_unrelated,
    clippy::wildcard_enum_match_arm,
    reason = "Codex provider passthrough preserves vendor JSON without a closed schema"
)]

use std::collections::BTreeSet;

use serde_json::{Map, Value};

const ADDITIONAL_TOOLS: &str = "additional_tools";

#[derive(Debug, Clone, PartialEq)]
pub struct HoistedAdditionalTools {
    pub input: Value,
    pub tools: Vec<Value>,
    pub hoisted: Vec<Value>,
    pub debug: Option<String>,
}

pub fn normalize_codex_input_items(input: &Value) -> (Value, Vec<String>) {
    let Value::Array(items) = input else {
        return (input.clone(), Vec::new());
    };
    let mut rewritten_types = BTreeSet::new();
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        let (normalized_item, item_type) = normalize_item(item);
        if let Some(item_type) = item_type {
            rewritten_types.insert(item_type.to_owned());
        }
        if let Some(normalized_item) = normalized_item {
            normalized.push(normalized_item);
        }
    }
    (
        Value::Array(normalized),
        rewritten_types.into_iter().collect(),
    )
}

pub fn drop_unsupported_tools(
    tools: &[Value],
    supported_types: &BTreeSet<String>,
) -> (Vec<Value>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped_types = BTreeSet::new();
    for tool in tools {
        let Some(object) = tool.as_object() else {
            kept.push(tool.clone());
            continue;
        };
        let supported = object
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| supported_types.contains(value));
        if supported {
            kept.push(tool.clone());
        } else {
            dropped_types.insert(
                object
                    .get("type")
                    .map_or_else(|| "None".to_owned(), value_name),
            );
        }
    }
    (kept, dropped_types.into_iter().collect())
}

pub fn hoist_additional_tools(input: &Value, tools: Option<&[Value]>) -> HoistedAdditionalTools {
    let existing = tools.map_or_else(Vec::new, <[Value]>::to_vec);
    if !input.is_array() {
        return HoistedAdditionalTools {
            input: input.clone(),
            tools: existing,
            hoisted: Vec::new(),
            debug: None,
        };
    }
    let items = input
        .as_array()
        .map_or(0, |values| additional_tools_count(values));
    if items == 0 {
        return HoistedAdditionalTools {
            input: input.clone(),
            tools: existing,
            hoisted: Vec::new(),
            debug: None,
        };
    }
    let (remaining, hoisted) = split_additional_tools(input);
    let debug = format!(
        "Responses API: hoisting {} tool(s) out of {} 'additional_tools' input item(s) into the top-level tools param.",
        hoisted.len(),
        items
    );
    let mut all_tools = existing;
    all_tools.extend(hoisted.iter().cloned());
    HoistedAdditionalTools {
        input: Value::Array(remaining),
        tools: all_tools,
        hoisted,
        debug: Some(debug),
    }
}

fn additional_tools_count(items: &[Value]) -> usize {
    let mut count: usize = 0;
    for item in items {
        if is_additional_tools(item) {
            count = count.saturating_add(1);
        }
    }
    count
}

fn split_additional_tools(input: &Value) -> (Vec<Value>, Vec<Value>) {
    let mut hoisted = Vec::new();
    let mut remaining = Vec::new();
    for item in input.as_array().into_iter().flatten() {
        if is_additional_tools(item) {
            if let Some(item_tools) = item.get("tools").and_then(Value::as_array) {
                hoisted.extend(item_tools.iter().filter(|tool| tool.is_object()).cloned());
            }
        } else {
            remaining.push(item.clone());
        }
    }
    (remaining, hoisted)
}

fn normalize_item(item: &Value) -> (Option<Value>, Option<&str>) {
    let Some(object) = item.as_object() else {
        return (Some(item.clone()), None);
    };
    match object.get("type").and_then(Value::as_str) {
        Some("agent_message") => (normalize_agent_message(object), Some("agent_message")),
        Some("context_compaction") => (normalize_compaction(object), Some("context_compaction")),
        Some("local_shell_call") => (normalize_local_shell(object), Some("local_shell_call")),
        _ => (Some(item.clone()), None),
    }
}

fn normalize_agent_message(item: &Map<String, Value>) -> Option<Value> {
    let content = item.get("content")?.as_array()?;
    let mut text = String::new();
    for block in content {
        let Some(block_object) = block.as_object() else {
            continue;
        };
        let selected_value = truthy_field(block_object, "text")
            .or_else(|| truthy_field(block_object, "encrypted_content"));
        if let Some(selected_value) = selected_value {
            text.push_str(&value_to_python_string(selected_value));
        }
    }
    if text.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}),
    )
}

fn normalize_compaction(item: &Map<String, Value>) -> Option<Value> {
    let encrypted = nonempty_field(item, "encrypted_content")?;
    Some(serde_json::json!({"type":"compaction","encrypted_content":encrypted}))
}

fn normalize_local_shell(item: &Map<String, Value>) -> Option<Value> {
    let call_id = nonempty_field(item, "call_id")?;
    let arguments = item
        .get("action")
        .filter(|value| value.is_object())
        .map_or_else(|| "{}".to_owned(), python_json);
    Some(
        serde_json::json!({"type":"function_call","call_id":call_id,"name":"local_shell","arguments":arguments}),
    )
}

fn is_additional_tools(item: &Value) -> bool {
    let item_type = item.get("type").and_then(Value::as_str);
    item_type == Some(ADDITIONAL_TOOLS)
}

fn nonempty_field<'value>(item: &'value Map<String, Value>, field: &str) -> Option<&'value str> {
    let value = item.get(field).and_then(Value::as_str)?;
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn truthy_field<'value>(item: &'value Map<String, Value>, field: &str) -> Option<&'value Value> {
    item.get(field).filter(|value| is_truthy(value))
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

fn value_name(value: &Value) -> String {
    if let Some(name) = value.as_str() {
        return name.to_owned();
    }
    value.to_string()
}

fn value_to_python_string(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    value.to_string()
}

fn python_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let body = object
                .iter()
                .map(|(key, value)| format!("{}: {}", json_string(key), python_json(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{body}}}")
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_json)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => value.to_string(),
    }
}

fn json_string(value: &str) -> String {
    if let Ok(encoded) = serde_json::to_string(value) {
        return encoded;
    }
    "\"\"".to_owned()
}

#[path = "codex_history_tools/tests.rs"]
mod tests;
