use crate::content_detection::detect_content_type;
use crate::extract_key::content_to_string;
use serde_json::Value;

pub fn stub_message(message: &Value, key: &str) -> Value {
    let content = message
        .get("content")
        .map(content_to_string)
        .unwrap_or_default();
    let line_count = content.lines().count().max(1);
    let content_type = detect_content_type(&content);
    let stub = format!(
        "[Compressed: {key} — {line_count} lines, {content_type}. Use litellm_content_retrieve tool to get full content.]"
    );
    let mut result = message.as_object().cloned().unwrap_or_default();
    result.insert("content".to_owned(), Value::String(stub));
    Value::Object(result)
}

mod tests;
