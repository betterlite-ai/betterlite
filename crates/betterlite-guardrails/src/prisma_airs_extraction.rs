mod guardrails;

use serde_json::Value;

pub fn extract_text_from_messages(messages: &[Value]) -> String {
    for message in messages.iter().rev() {
        let Some(role) = message.get("role").and_then(Value::as_str) else {
            continue;
        };
        if role != "user" && role != "developer" {
            continue;
        }
        if let Some(content) = message.get("content") {
            if let Some(text) = content_text(content) {
                return text;
            }
        }
    }
    String::new()
}

fn flatten_content_list(parts: &[Value]) -> String {
    parts
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn content_text(content: &Value) -> Option<String> {
    let text = content
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| content.as_array().map(|parts| flatten_content_list(parts)))?;
    if text.is_empty() {
        return None;
    }
    Some(text)
}

pub fn extract_response_text(response: &Value) -> String {
    let Some(choices) = response.get("choices").and_then(Value::as_array) else {
        return String::new();
    };
    let mut text = Vec::new();
    for message in choices.iter().filter_map(|choice| choice.get("message")) {
        if let Some(content) = message.get("content").and_then(content_text) {
            text.push(content);
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for arguments in calls
                .iter()
                .filter_map(|call| call.pointer("/function/arguments"))
            {
                text.push(value_as_text(arguments));
            }
        }
        if let Some(arguments) = message.pointer("/function_call/arguments") {
            text.push(value_as_text(arguments));
        }
    }
    text.join(" ")
}

fn value_as_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}
