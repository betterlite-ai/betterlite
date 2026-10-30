use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

mod tests;

#[must_use]
pub fn expand_for_classification(messages: &[Value]) -> Vec<Value> {
    let mut expanded = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let content = message.get("content");
        if role == "tool" {
            if let Some(call_id) = message.get("tool_call_id").and_then(Value::as_str) {
                expanded.push(json!({"role":"developer","content":{"type":"tool_result","tool_use_id":call_id,"content":flatten_content(content)}}));
            }
            continue;
        }
        if let Some(content) = classification_content(content) {
            expanded.push(json!({"role":role,"content":content}));
        }
        if role == "assistant" {
            if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    if let (Some(id), Some(name)) = (
                        call.get("id").and_then(Value::as_str),
                        call.pointer("/function/name").and_then(Value::as_str),
                    ) {
                        let input = call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .and_then(|raw| {
                                serde_json::from_str::<Value>(raw)
                                    .ok()
                                    .filter(Value::is_object)
                            })
                            .unwrap_or_else(|| {
                                json!({"arguments": call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("")})
                            });
                        expanded.push(json!({"role":"model","content":{"type":"tool_use","id":id,"name":name,"input":input}}));
                    }
                }
            }
        }
    }
    expanded
}

fn flatten_content(content: Option<&Value>) -> Value {
    match content {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(parts)) => {
            let text = parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            Value::String(text)
        }
        Some(value) => value.clone(),
        None => Value::String(String::new()),
    }
}

fn classification_content(content: Option<&Value>) -> Option<Value> {
    let flattened = flatten_content(content);
    match &flattened {
        Value::String(text) if text.is_empty() => None,
        Value::Null => None,
        _ => Some(flattened),
    }
}

#[must_use]
pub fn map_masked_messages_back(original: &[Value], masked: &[Value]) -> Vec<Value> {
    let mut tool_use = BTreeMap::new();
    let mut tool_result = BTreeMap::new();
    let mut texts = Vec::new();
    for message in masked {
        match message.get("content") {
            Some(Value::String(text)) => texts.push(text.clone()),
            Some(Value::Object(content))
                if content.get("type").and_then(Value::as_str) == Some("tool_use") =>
            {
                if let Some(id) = content.get("id").and_then(Value::as_str) {
                    tool_use.insert(id.to_owned(), content.clone());
                }
            }
            Some(Value::Object(content))
                if content.get("type").and_then(Value::as_str) == Some("tool_result") =>
            {
                if let Some(id) = content.get("tool_use_id").and_then(Value::as_str) {
                    tool_result.insert(
                        id.to_owned(),
                        content.get("content").cloned().unwrap_or(Value::Null),
                    );
                }
            }
            _ => {}
        }
    }
    let count = original
        .iter()
        .filter(|message| {
            message.get("role").and_then(Value::as_str) != Some("tool")
                && text_content_exists(message.get("content"))
        })
        .count();
    let mut cursor = 0;
    original
        .iter()
        .map(|message| {
            map_one_message(
                message,
                &texts,
                &mut cursor,
                count == texts.len(),
                &tool_use,
                &tool_result,
            )
        })
        .collect()
}

fn map_one_message(
    message: &Value,
    texts: &[String],
    cursor: &mut usize,
    apply_text: bool,
    tool_use: &BTreeMap<String, Map<String, Value>>,
    tool_result: &BTreeMap<String, Value>,
) -> Value {
    let mut mapped = message.as_object().cloned().unwrap_or_default();
    if message.get("role").and_then(Value::as_str) == Some("tool") {
        if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
            if let Some(content) = tool_result.get(id) {
                mapped.insert("content".to_owned(), content.clone());
            }
        }
    } else if apply_text && text_content_exists(message.get("content")) {
        if let Some(text) = texts.get(*cursor) {
            mapped.insert("content".to_owned(), Value::String(text.clone()));
            *cursor += 1;
        }
    }
    if message.get("role").and_then(Value::as_str) == Some("assistant") {
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            mapped.insert(
                "tool_calls".to_owned(),
                Value::Array(update_tool_calls(calls, tool_use)),
            );
        }
    }
    Value::Object(mapped)
}

fn text_content_exists(content: Option<&Value>) -> bool {
    match content {
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_)) => true,
        _ => false,
    }
}

#[must_use]
pub fn update_tool_calls(
    calls: &[Value],
    masked: &BTreeMap<String, Map<String, Value>>,
) -> Vec<Value> {
    let mut updated = Vec::with_capacity(calls.len());
    for call in calls {
        let value = {
            let Some(id) = call.get("id").and_then(Value::as_str) else {
                updated.push(call.clone());
                continue;
            };
            let Some(input) = masked.get(id).and_then(|value| value.get("input")) else {
                updated.push(call.clone());
                continue;
            };
            let mut result = call.as_object().cloned().unwrap_or_default();
            let mut function = call
                .get("function")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            function.insert("arguments".to_owned(), Value::String(input.to_string()));
            result.insert("function".to_owned(), Value::Object(function));
            Value::Object(result)
        };
        updated.push(value);
    }
    updated
}

pub fn apply_model_response(response: &mut Value, masked: &[Value]) {
    let mut tool_use = BTreeMap::new();
    let texts = masked
        .iter()
        .filter_map(|message| match message.get("content") {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Object(content))
                if content.get("type").and_then(Value::as_str) == Some("tool_use") =>
            {
                content.get("id").and_then(Value::as_str).map(|id| {
                    tool_use.insert(id.to_owned(), content.clone());
                    String::new()
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut cursor = 0;
    if let Some(choices) = response.get_mut("choices").and_then(Value::as_array_mut) {
        let text_count = choices
            .iter()
            .filter(|choice| {
                choice
                    .pointer("/message/content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty())
                    || choice
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.is_empty())
            })
            .count();
        let apply_text = text_count == texts.len();
        for choice in choices {
            if let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) {
                if apply_text && message.get("content").and_then(Value::as_str).is_some() {
                    if let Some(text) = texts.get(cursor) {
                        message.insert("content".to_owned(), Value::String(text.clone()));
                        cursor += 1;
                    }
                }
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    message.insert(
                        "tool_calls".to_owned(),
                        Value::Array(update_tool_calls(calls, &tool_use)),
                    );
                }
            } else if apply_text && choice.get("text").and_then(Value::as_str).is_some() {
                if let Some(text) = texts.get(cursor) {
                    if let Some(object) = choice.as_object_mut() {
                        object.insert("text".to_owned(), Value::String(text.clone()));
                    }
                    cursor += 1;
                }
            }
        }
    }
}
