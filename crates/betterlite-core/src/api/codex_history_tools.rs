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
        let (rewritten, rewritten_type) = normalize_item(item);
        if let Some(rewritten_type) = rewritten_type {
            rewritten_types.insert(rewritten_type.to_owned());
        }
        if let Some(rewritten) = rewritten {
            normalized.push(rewritten);
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
        return result(input, existing, Vec::new(), None);
    }
    let items = input.as_array().map_or(0, |values| {
        values
            .iter()
            .filter(|item| is_additional_tools(item))
            .count()
    });
    if items == 0 {
        return result(input, existing, Vec::new(), None);
    }
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
    let debug = format!(
        "Responses API: hoisting {} tool(s) out of {} 'additional_tools' input item(s) into the top-level tools param.",
        hoisted.len(),
        items
    );
    let mut all_tools = existing;
    all_tools.extend(hoisted.iter().cloned());
    result(&Value::Array(remaining), all_tools, hoisted, Some(debug))
}

fn result(
    input: &Value,
    tools: Vec<Value>,
    hoisted: Vec<Value>,
    debug: Option<String>,
) -> HoistedAdditionalTools {
    HoistedAdditionalTools {
        input: input.clone(),
        tools,
        hoisted,
        debug,
    }
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
        let Some(block) = block.as_object() else {
            continue;
        };
        let value = block
            .get("text")
            .filter(|value| is_truthy(value))
            .or_else(|| {
                block
                    .get("encrypted_content")
                    .filter(|value| is_truthy(value))
            });
        if let Some(value) = value {
            text.push_str(&value_to_python_string(value));
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
    let encrypted = item
        .get("encrypted_content")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    Some(serde_json::json!({"type":"compaction","encrypted_content":encrypted}))
}

fn normalize_local_shell(item: &Map<String, Value>) -> Option<Value> {
    let call_id = item
        .get("call_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let arguments = item
        .get("action")
        .filter(|value| value.is_object())
        .map_or_else(|| "{}".to_owned(), python_json);
    Some(
        serde_json::json!({"type":"function_call","call_id":call_id,"name":"local_shell","arguments":arguments}),
    )
}

fn is_additional_tools(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some(ADDITIONAL_TOOLS)
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
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToOwned::to_owned)
}

fn value_to_python_string(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToOwned::to_owned)
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
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

#[cfg(test)]
#[path = "codex_history_tools/tests.rs"]
mod tests;
