use serde_json::{Value, json};

const EMPTY_TEXT: &str = "[System: Empty message content sanitised to satisfy protocol]";

pub fn sanitize_empty_text_content(message: &Value) -> Value {
    let Some(object) = message.as_object() else {
        return message.clone();
    };
    if !matches!(
        object.get("role").and_then(Value::as_str),
        Some("user" | "assistant")
    ) {
        return message.clone();
    }
    match object.get("content") {
        Some(Value::String(text)) if text.trim().is_empty() => {
            let mut sanitized = object.clone();
            sanitized.insert("content".to_owned(), Value::String(EMPTY_TEXT.to_owned()));
            Value::Object(sanitized)
        }
        Some(Value::Array(blocks)) => {
            let mut rewritten = false;
            let content = blocks
                .iter()
                .map(|block| {
                    let Some(block_object) = block.as_object() else {
                        return block.clone();
                    };
                    if block_object.get("type") != Some(&json!("text"))
                        || block_object
                            .get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|text| !text.trim().is_empty())
                    {
                        return block.clone();
                    }
                    rewritten = true;
                    let mut replaced = block_object.clone();
                    replaced.insert("text".to_owned(), Value::String(EMPTY_TEXT.to_owned()));
                    Value::Object(replaced)
                })
                .collect::<Vec<_>>();
            if rewritten {
                let mut sanitized = object.clone();
                sanitized.insert("content".to_owned(), Value::Array(content));
                Value::Object(sanitized)
            } else {
                message.clone()
            }
        }
        _ => message.clone(),
    }
}
