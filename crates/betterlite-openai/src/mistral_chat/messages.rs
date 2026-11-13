use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use serde_json::Value;

use super::MistralChatError;
use crate::chat::{ChatMessage, MessageContent, Role};

const ASSISTANT_FIELDS: [&str; 6] = [
    "tool_calls",
    "function_call",
    "reasoning_content",
    "cache_control",
    "thinking_blocks",
    "reasoning_items",
];

#[must_use]
pub fn strip_output_only_fields(message: &ChatMessage) -> ChatMessage {
    let mut stripped = message.clone();
    if stripped.role == Role::Assistant {
        stripped.passthrough.shift_remove("reasoning_content");
        stripped.passthrough.shift_remove("thinking_blocks");
    }
    stripped
}

#[must_use]
pub fn handle_name_in_message(mut message: Message) -> Message {
    if let Some(name) = message.name.take()
        && message.role == "tool"
        && !name.trim().is_empty()
    {
        message.name = Some(name);
    }
    message
}

#[must_use]
pub fn handle_tool_call_message(mut message: Message) -> Message {
    let projected = match message.provider_specific_fields.get("tool_calls") {
        Some(Json::List(calls)) => calls
            .iter()
            .map(|call| {
                let fields = call.as_object();
                let pick = |name: &str| {
                    fields
                        .and_then(|given| given.get(name))
                        .cloned()
                        .unwrap_or(Json::Null)
                };
                Json::Object(Object::from_iter([
                    (String::from("id"), pick("id")),
                    (String::from("type"), Json::Text(String::from("function"))),
                    (String::from("function"), pick("function")),
                ]))
            })
            .collect(),
        _ => return message,
    };
    message
        .provider_specific_fields
        .insert(String::from("tool_calls"), Json::List(projected));
    message
}

#[must_use]
pub fn is_empty_assistant_message(message: &Message) -> bool {
    let empty_content = match &message.content {
        Some(Content::Text(text)) => text.is_empty(),
        Some(Content::Parts(_)) => false,
        None => true,
    };
    if !empty_content || message.name.is_some() {
        return false;
    }
    !ASSISTANT_FIELDS.iter().any(|field| {
        message
            .provider_specific_fields
            .get(*field)
            .is_some_and(|value| !matches!(value, Json::Null))
    })
}

pub fn handle_message_with_file(messages: &mut [ChatMessage]) -> Result<(), MistralChatError> {
    for message in messages {
        let Some(MessageContent::Parts(parts)) = &mut message.content else {
            continue;
        };
        for part in parts.iter_mut() {
            if part.kind != "file" {
                continue;
            }
            let lifted = match part.fields.get("file") {
                None => None,
                Some(file) => {
                    let Some(fields) = file.as_object() else {
                        return Err(MistralChatError::FileBlockShape);
                    };
                    match fields.get("file_id") {
                        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
                        _ => None,
                    }
                }
            };
            if let Some(id) = lifted {
                part.fields
                    .insert(String::from("file_id"), Value::String(id));
                part.fields.shift_remove("file");
            }
        }
    }
    Ok(())
}

pub(super) fn has_media_blocks(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|message| match &message.content {
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .any(|part| matches!(part.kind.as_str(), "image_url" | "file")),
        Some(MessageContent::Text(_)) | None => false,
    })
}
