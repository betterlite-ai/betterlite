use serde::Serialize;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;

#[derive(Debug, Clone, Serialize)]
pub struct TextUpdate {
    pub response: Value,
    pub replaced: bool,
}

#[must_use]
pub fn set_tool_response_text(response: &Json, text: &str) -> TextUpdate {
    let value = serde_json::from_str(response_json(response).as_str()).unwrap_or(Value::Null);
    let (response, replaced) = rewrite_value(value, text);
    TextUpdate { response, replaced }
}

fn response_json(response: &Json) -> String {
    match response {
        Json::Null => "null".to_owned(),
        Json::Bool(value) => value.to_string(),
        Json::Number(value) => value.clone(),
        Json::Text(value) => serde_json::to_string(value).unwrap_or_default(),
        Json::List(items) => format!("[{}]", raw_items(items)),
        Json::Object(fields) => serde_json::to_string(fields).unwrap_or_default(),
    }
}

fn raw_items(items: &[Box<RawValue>]) -> String {
    items
        .iter()
        .map(|item| item.get())
        .collect::<Vec<_>>()
        .join(",")
}

fn rewrite_value(mut value: Value, text: &str) -> (Value, bool) {
    if let Some(fields) = value.as_object_mut()
        && let Some(inner) = fields.get_mut("mcp_tool_call_response")
        && !inner.is_null()
    {
        let (updated, replaced) = rewrite_value(inner.clone(), text);
        *inner = updated;
        return (value, replaced);
    }
    if let Some(fields) = value.as_object_mut() {
        let model = fields.get("content").is_some_and(Value::is_array);
        let replaced = fields
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .map_or(false, |items| rewrite_items(items, text));
        if model {
            let key = if fields.contains_key("structured_content") {
                "structured_content"
            } else {
                "structuredContent"
            };
            fields.insert(key.to_owned(), serde_json::json!({"result": text}));
            return (value, true);
        }
        if let Some(target) = fields.get_mut("result").and_then(Value::as_object_mut) {
            if target.contains_key("structured_content") {
                target.insert(
                    "structured_content".to_owned(),
                    serde_json::json!({"result": text}),
                );
                return (value, true);
            }
            if target.contains_key("structuredContent") {
                target.insert(
                    "structuredContent".to_owned(),
                    serde_json::json!({"result": text}),
                );
                return (value, true);
            }
        } else {
            if fields.contains_key("structured_content") {
                fields.insert(
                    "structured_content".to_owned(),
                    serde_json::json!({"result": text}),
                );
                return (value, true);
            }
            if fields.contains_key("structuredContent") {
                fields.insert(
                    "structuredContent".to_owned(),
                    serde_json::json!({"result": text}),
                );
                return (value, true);
            }
        }
        return (value, replaced);
    }
    if let Some(items) = value.as_array_mut() {
        if pair_list(items) {
            let mut content_replaced = false;
            for pair in items.iter_mut() {
                let Some(parts) = pair.as_array_mut() else {
                    continue;
                };
                let Some(key) = parts.first().and_then(Value::as_str) else {
                    continue;
                };
                if key == "content" {
                    if let Some(content) = parts.get_mut(1).and_then(Value::as_array_mut) {
                        content_replaced = rewrite_items(content, text);
                    }
                }
            }
            for pair in items.iter_mut() {
                let Some(parts) = pair.as_array_mut() else {
                    continue;
                };
                if parts
                    .first()
                    .and_then(Value::as_str)
                    .is_some_and(|key| matches!(key, "structuredContent" | "structured_content"))
                    && let Some(mirror) = parts.get_mut(1)
                {
                    *mirror = serde_json::json!({"result": text});
                    return (value, true);
                }
            }
            return (value, content_replaced);
        }
        return (value, rewrite_items(items, text));
    }
    (value, false)
}

fn pair_list(items: &[Value]) -> bool {
    !items.is_empty()
        && items.iter().all(|item| {
            item.as_array()
                .is_some_and(|pair| pair.len() == 2 && pair.first().is_some_and(Value::is_string))
        })
}

fn rewrite_items(items: &mut [Value], text: &str) -> bool {
    let mut replaced = false;
    for item in items {
        if let Some(fields) = item.as_object_mut()
            && fields.get("type").and_then(Value::as_str) == Some("text")
        {
            fields.insert("text".to_owned(), Value::String(text.to_owned()));
            replaced = true;
        }
    }
    replaced
}

mod tests;
