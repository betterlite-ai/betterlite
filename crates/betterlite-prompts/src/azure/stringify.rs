use crate::arguments::json::{Json, Object};
use crate::azure::error::AzureError;
use crate::media::limits::MAX_MESSAGES;
use crate::message_content::{Content, Message};

const TOOL: &str = "tool";
const RESULT: &str = "result";

pub fn stringify_json_tool_call_content(messages: &[Message]) -> Result<Vec<Message>, AzureError> {
    if messages.len() > MAX_MESSAGES {
        return Err(AzureError::TooManyMessages {
            limit: MAX_MESSAGES,
        });
    }
    let mut converted = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        converted.push(stringify_message(message, index)?);
    }
    Ok(converted)
}

fn stringify_message(message: &Message, index: usize) -> Result<Message, AzureError> {
    let mut converted = message.clone();
    if converted.role != TOOL {
        return Ok(converted);
    }
    let text = match &converted.content {
        Some(Content::Text(text)) => text.clone(),
        Some(Content::Parts(_)) => return Ok(converted),
        None => return Err(AzureError::MissingToolContent { index }),
    };
    if serde_json::from_str::<Json>(&text).is_err() {
        converted.content = Some(Content::Text(wrapped_result(&text)?));
    }
    Ok(converted)
}

fn wrapped_result(text: &str) -> Result<String, AzureError> {
    let mut fields = Object::new();
    fields.insert(RESULT.to_owned(), Json::Text(text.to_owned()));
    serde_json::to_string(&Json::Object(fields)).map_err(|source| AzureError::Serialize { source })
}
