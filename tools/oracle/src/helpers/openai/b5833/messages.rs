use serde_json::{Value, json};

use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::groq_chat::error::GroqChatError;
use betterlite_openai::groq_chat::messages::{MessageTransform, transform_messages};

use super::Call;

pub(super) fn message_one(call: &Call) -> Result<Value, GroqChatError> {
    let downloads = RefuseDownloads;
    let transform = MessageTransform {
        model: call.model.as_deref().unwrap_or(""),
        messages: &call.messages,
        environment: &call.environment,
        downloads: &downloads,
    };
    let messages = transform_messages(&transform)?;
    Ok(json!({"messages": messages}))
}
