use serde_json::{Map, Value};

pub const ANTHROPIC_CALL_TYPES: [&str; 1] = ["anthropic_messages"];
pub const SUPPORTED_CALL_TYPES: [&str; 3] = ["completion", "acompletion", "anthropic_messages"];

pub fn normalize_call_type(call_type: &str) -> String {
    if call_type.is_empty() {
        String::new()
    } else {
        call_type.to_owned()
    }
}

pub fn is_anthropic_call_type(call_type: &str) -> bool {
    let is_anthropic = matches!(call_type, "anthropic_messages");
    if is_anthropic {
        return true;
    }
    false
}

pub fn content_to_text(content: &Value) -> String {
    let mut parts = Vec::new();
    let mut pending = vec![content];
    while let Some(item) = pending.pop() {
        match item {
            Value::String(text) => parts.push(text.clone()),
            Value::Array(items) => pending.extend(items.iter().rev()),
            Value::Object(object) => {
                if object.get("type") == Some(&Value::String("text".to_owned())) {
                    if let Some(Value::String(text)) = object.get("text") {
                        parts.push(text.clone());
                    }
                } else if object.get("type") == Some(&Value::String("tool_result".to_owned()))
                    && let Some(nested) = object.get("content")
                {
                    pending.push(nested);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    parts.join(" ")
}

pub fn normalize_messages(
    messages: &[Value],
    call_type: &str,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    if !SUPPORTED_CALL_TYPES.contains(&call_type) {
        return Err(format!(
            "Unsupported call_type={call_type:?} for compression. Expected one of: {:?}.",
            SUPPORTED_CALL_TYPES
        ));
    }
    let original = messages.to_vec();
    let normalized = original
        .iter()
        .map(|message| {
            let mut copy = message.as_object().cloned().unwrap_or_else(Map::new);
            let content = copy.get("content").unwrap_or(&Value::Null);
            copy.insert(
                "content".to_owned(),
                Value::String(content_to_text(content)),
            );
            Value::Object(copy)
        })
        .collect();
    Ok((normalized, original))
}

pub fn extract_last_user_message(messages: &[Value]) -> String {
    let last_user = messages
        .iter()
        .rev()
        .find(|message| message.get("role") == Some(&Value::String("user".to_owned())))
        .and_then(|message| message.get("content"))
        .map_or_else(String::new, content_to_text);
    if last_user.is_empty() {
        String::new()
    } else {
        last_user
    }
}

mod tests;
