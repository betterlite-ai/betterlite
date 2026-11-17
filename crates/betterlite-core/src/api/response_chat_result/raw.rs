use serde_json::{Map, Value};

use super::choices::keep_annotations;
use super::error::ResponseChatResultError as Error;
use super::records::{ChatChoice, ChatMessage, ChatRole};
use crate::api::response_chat_content::annotations::AnnotationValue;
use crate::api::responses_input_content::is_truthy;

// a raw message contributes only its first output_text block, and its role decides whether it is a choice
pub(super) fn raw_message_choice(
    item: &Map<String, Value>,
    index: usize,
) -> Result<Option<ChatChoice>, Error> {
    if item.get("type").and_then(Value::as_str) != Some("message") {
        return Ok(None);
    }
    let content = match item.get("content") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(content)) => content,
        Some(_) => return Err(Error::RawField { field: "content" }),
    };
    for content_item in content {
        let Value::Object(part) = content_item else {
            continue;
        };
        if part.get("type").and_then(Value::as_str) != Some("output_text") {
            continue;
        }
        let message = ChatMessage {
            content: Some(raw_text(part)?),
            role: raw_role(item.get("role"))?,
            tool_calls: None,
            function_call: None,
            provider_specific_fields: None,
            reasoning_content: None,
            reasoning_items: None,
            annotations: raw_annotations(part.get("annotations"))?,
        };
        return Ok(Some(ChatChoice {
            finish_reason: "stop".to_owned(),
            index,
            message,
        }));
    }
    Ok(None)
}

// Message turns a falsy text into an empty string and rejects one that is not a string
fn raw_text(part: &Map<String, Value>) -> Result<String, Error> {
    match part.get("text") {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) if is_truthy(Some(other)) => Err(Error::RawField { field: "text" }),
        None | Some(_) => Ok(String::new()),
    }
}

// Message turns a missing, null or empty role into assistant and rejects one outside its literal set
fn raw_role(value: Option<&Value>) -> Result<ChatRole, Error> {
    match value {
        None | Some(Value::Null) => Ok(ChatRole::Assistant),
        Some(Value::String(role)) => chat_role(role),
        Some(_) => Err(Error::RawField { field: "role" }),
    }
}

fn chat_role(role: &str) -> Result<ChatRole, Error> {
    match role {
        "" | "assistant" => Ok(ChatRole::Assistant),
        "user" => Ok(ChatRole::User),
        "system" => Ok(ChatRole::System),
        "tool" => Ok(ChatRole::Tool),
        "function" => Ok(ChatRole::Function),
        _ => Err(Error::RawField { field: "role" }),
    }
}

// every annotation object is forwarded, and no citation kind is filtered out here
fn raw_annotations(value: Option<&Value>) -> Result<Option<Vec<Value>>, Error> {
    let items = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(items)) => items,
        Some(_) => {
            return Err(Error::RawField {
                field: "annotations",
            });
        }
    };
    let values: Vec<AnnotationValue> = items
        .iter()
        .filter(|item| item.is_object())
        .map(|item| AnnotationValue::Dict(item.clone()))
        .collect();
    Ok(keep_annotations(&values))
}
