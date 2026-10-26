use serde_json::{Map, Value};
use std::str::FromStr as _;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompressionError {
    #[error(
        "Unsupported call_type='{call_type}' for compression. Expected one of: ['acompletion', 'anthropic_messages', 'completion']."
    )]
    UnsupportedCallType {
        call_type: String,
        source: strum::ParseError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum CallType {
    Completion,
    Acompletion,
    AnthropicMessages,
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
) -> Result<(CallType, Vec<Value>, Vec<Value>), CompressionError> {
    let parsed =
        CallType::from_str(call_type).map_err(|source| CompressionError::UnsupportedCallType {
            call_type: call_type.to_owned(),
            source,
        })?;
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
    Ok((parsed, normalized, original))
}

pub fn extract_last_user_message(messages: &[Value]) -> String {
    for message in messages.iter().rev() {
        if message.get("role") == Some(&Value::String("user".to_owned())) {
            return content_to_text(message.get("content").unwrap_or(&Value::Null));
        }
    }
    String::new()
}

mod tests;
