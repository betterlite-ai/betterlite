use betterlite_prompts::message_content::{Message, convert_content_list_to_str};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PromptError {
    #[error("{0}")]
    MalformedContent(#[from] serde_json::Error),
}

// Python's bool is an int subclass, and all() over an empty list is vacuously true, so an empty list qualifies too.
#[must_use]
pub fn is_tokens_or_list_of_tokens(value: &Value) -> bool {
    let Value::Array(items) = value else {
        return false;
    };
    let is_int_like = |item: &Value| {
        matches!(item, Value::Bool(_)) || item.as_i64().is_some() || item.as_u64().is_some()
    };
    items.iter().all(is_int_like)
        || items
            .iter()
            .all(|item| matches!(item, Value::Array(inner) if inner.iter().all(is_int_like)))
}

// The one-message branch also requires the content to be truthy, so an empty list still falls through to the string conversion below.
fn is_nonempty_token_list(content: Option<&Value>) -> bool {
    let Some(value @ Value::Array(items)) = content else {
        return false;
    };
    !items.is_empty() && is_tokens_or_list_of_tokens(value)
}

pub fn transform_prompt(messages: &[Value]) -> Result<Value, PromptError> {
    if let [only] = messages {
        let content = only.get("content");
        if is_nonempty_token_list(content) {
            return Ok(content.cloned().unwrap_or(Value::Null));
        }
        let message: Message = serde_json::from_value(only.clone())?;
        return Ok(Value::String(convert_content_list_to_str(&message)));
    }
    let mut converted = Vec::with_capacity(messages.len());
    for raw_message in messages {
        let message: Message = serde_json::from_value(raw_message.clone())?;
        converted.push(Value::String(convert_content_list_to_str(&message)));
    }
    Ok(Value::Array(converted))
}
