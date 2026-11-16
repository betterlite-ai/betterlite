use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyFailure {
    pub status: u16,
    pub message: Option<String>,
}

// LiteLLM raises for a meaningful error object inside an otherwise successful reply
#[must_use]
pub fn reply_error(reply: &Value) -> Option<ReplyFailure> {
    let error = reply.get("error").filter(|error| !error.is_null())?;
    match error {
        Value::String(text) if text.is_empty() => None,
        Value::Object(fields) => object_error(fields),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Some(ReplyFailure {
                status: 422,
                message: None,
            })
        }
    }
}

fn object_error(fields: &Map<String, Value>) -> Option<ReplyFailure> {
    let message = fields.get("message");
    let code = fields.get("code");
    if message.is_none_or(is_blank) && code.is_none_or(Value::is_null) {
        return None;
    }
    let status = code
        .and_then(Value::as_u64)
        .and_then(|number| u16::try_from(number).ok())
        .unwrap_or(422);
    Some(ReplyFailure {
        status,
        message: message_text(message),
    })
}

// None stands for LiteLLM's own "Error in response object" text
fn message_text(message: Option<&Value>) -> Option<String> {
    match message {
        Some(Value::String(text)) => Some(text.clone()),
        Some(other) if !is_blank(other) => Some(other.to_string()),
        _ => None,
    }
}

fn is_blank(value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    if let Some(flag) = value.as_bool() {
        return !flag;
    }
    if let Some(number) = value.as_f64() {
        return number == 0.0;
    }
    if let Some(text) = value.as_str() {
        return text.is_empty();
    }
    if let Some(items) = value.as_array() {
        return items.is_empty();
    }
    value.as_object().is_some_and(Map::is_empty)
}

mod tests;
