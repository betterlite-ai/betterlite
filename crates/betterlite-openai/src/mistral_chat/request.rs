use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{
    Content, Message, convert_message_content_lists, strip_none_values_from_message,
};
use indexmap::IndexMap;
use serde_json::Value;

use super::{MistralChatError, PROVIDER};
use crate::chat::{ChatMessage, ContentPart, MessageContent, Role};
use crate::chat_content::{ChatContent, Mode, TransformedRequest, UrlDownloader};
use crate::mistral_options::REASONING_SYSTEM_PROMPT;

const REASONING_FLAG: &str = "_add_reasoning_prompt";
const CLIENT_METADATA: &str = "client_metadata";
const OUTPUT_ONLY_FIELDS: [&str; 2] = ["reasoning_content", "thinking_blocks"];
const ASSISTANT_FIELDS: [&str; 6] = [
    "tool_calls",
    "function_call",
    "reasoning_content",
    "cache_control",
    "thinking_blocks",
    "reasoning_items",
];

pub struct MessageTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub mode: Mode,
    pub downloads: &'request dyn UrlDownloader,
}

pub struct RequestTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub downloads: &'request dyn UrlDownloader,
}

struct Prepared {
    messages: Vec<ChatMessage>,
    media: bool,
}

pub fn transform_messages(
    request: &MessageTransform<'_>,
) -> Result<Vec<ChatMessage>, MistralChatError> {
    let prepared = prepared_messages(request.messages)?;
    let mut transformed = parent_transform(
        request.model,
        &prepared.messages,
        &Object::new(),
        request.mode,
        request.downloads,
    )?;
    if prepared.media {
        handle_message_with_file(&mut transformed.messages)?;
    }
    Ok(transformed.messages)
}

pub fn transform_request(request: &RequestTransform<'_>) -> Result<Object, MistralChatError> {
    let mut optional_params = request.optional_params.clone();
    let messages = add_reasoning_system_prompt_if_needed(
        request.model,
        request.messages,
        &mut optional_params,
    );
    optional_params.shift_remove(CLIENT_METADATA);
    let prepared = prepared_messages(&messages)?;
    let mut transformed = parent_transform(
        request.model,
        &prepared.messages,
        &optional_params,
        Mode::Sync,
        request.downloads,
    )?;
    if prepared.media {
        handle_message_with_file(&mut transformed.messages)?;
    }
    Ok(transformed.to_body()?)
}

// MistralConfig._transform_messages: media keeps the conversation as sent, text goes through the
// name, tool-call and empty-message cleanup first
fn prepared_messages(messages: &[ChatMessage]) -> Result<Prepared, MistralChatError> {
    let stripped: Vec<ChatMessage> = messages.iter().map(strip_output_only_fields).collect();
    let media = has_media_blocks(&stripped);
    if media {
        return Ok(Prepared {
            messages: stripped,
            media,
        });
    }
    let prompt: Vec<Message> = stripped
        .iter()
        .map(prompt_message)
        .collect::<Result<Vec<_>, _>>()?;
    let cleaned = cleaned_messages(prompt)?;
    Ok(Prepared {
        messages: cleaned,
        media,
    })
}

fn cleaned_messages(mut messages: Vec<Message>) -> Result<Vec<ChatMessage>, MistralChatError> {
    let converted = convert_message_content_lists(&mut messages);
    let mut cleaned = Vec::with_capacity(converted.len());
    for message in converted {
        let message = handle_name_in_message(message);
        let message = handle_tool_call_message(message);
        if is_empty_assistant_message(&message) {
            continue;
        }
        cleaned.push(chat_message(&strip_none_values_from_message(&message))?);
    }
    Ok(cleaned)
}

fn parent_transform(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    mode: Mode,
    downloads: &dyn UrlDownloader,
) -> Result<TransformedRequest, MistralChatError> {
    let environment = BTreeMap::new();
    let content = ChatContent {
        model,
        messages,
        optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads,
    };
    Ok(content.transform(mode)?)
}

#[must_use]
pub fn add_reasoning_system_prompt_if_needed(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &mut Object,
) -> Vec<ChatMessage> {
    let requested = optional_params
        .get(REASONING_FLAG)
        .is_some_and(Json::is_truthy);
    if !(model.to_lowercase().contains("magistral") && requested) {
        return messages.to_vec();
    }
    optional_params.shift_remove(REASONING_FLAG);
    let prompt = REASONING_SYSTEM_PROMPT;
    let mut updated = messages.to_vec();
    let Some(index) = updated
        .iter()
        .position(|message| message.role == Role::System)
    else {
        let mut inserted = Vec::with_capacity(updated.len() + 1);
        inserted.push(system_message(prompt));
        inserted.append(&mut updated);
        return inserted;
    };
    if let Some(message) = updated.get_mut(index) {
        let content = message.content.take();
        message.content = Some(with_reasoning_prefix(content, prompt));
    }
    updated
}

fn system_message(prompt: &str) -> ChatMessage {
    ChatMessage {
        role: Role::System,
        content: Some(MessageContent::Text(prompt.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn with_reasoning_prefix(content: Option<MessageContent>, prompt: &str) -> MessageContent {
    match content {
        Some(MessageContent::Parts(mut parts)) => {
            let mut fields = IndexMap::new();
            fields.insert(String::from("text"), Value::String(format!("{prompt}\n\n")));
            parts.insert(
                0,
                ContentPart {
                    kind: String::from("text"),
                    fields,
                },
            );
            MessageContent::Parts(parts)
        }
        Some(MessageContent::Text(text)) => MessageContent::Text(format!("{prompt}\n\n{text}")),
        None => MessageContent::Text(format!("{prompt}\n\n")),
    }
}

// _strip_output_only_fields drops the two response-only keys from assistant turns only
#[must_use]
pub fn strip_output_only_fields(message: &ChatMessage) -> ChatMessage {
    let mut stripped = message.clone();
    if stripped.role == Role::Assistant {
        for field in OUTPUT_ONLY_FIELDS {
            stripped.passthrough.shift_remove(field);
        }
    }
    stripped
}

// _handle_name_in_message keeps a name only on a tool message and only when it is not blank
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

// _handle_tool_call_message projects every tool call to Mistral's id/type/function record
#[must_use]
pub fn handle_tool_call_message(mut message: Message) -> Message {
    let projected = match message.provider_specific_fields.get("tool_calls") {
        Some(Json::List(calls)) => calls.iter().map(project_tool_call).collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => {
            return message;
        }
    };
    message
        .provider_specific_fields
        .insert(String::from("tool_calls"), Json::List(projected));
    message
}

fn project_tool_call(call: &Json) -> Json {
    let fields = call.as_object();
    Json::Object(Object::from_iter([
        (
            String::from("id"),
            fields
                .and_then(|given| given.get("id"))
                .cloned()
                .unwrap_or(Json::Null),
        ),
        (String::from("type"), Json::Text(String::from("function"))),
        (
            String::from("function"),
            fields
                .and_then(|given| given.get("function"))
                .cloned()
                .unwrap_or(Json::Null),
        ),
    ]))
}

// _is_empty_assistant_message reads the assistant field names on every role, role itself aside
#[must_use]
pub fn is_empty_assistant_message(message: &Message) -> bool {
    let content_is_empty = match &message.content {
        Some(Content::Text(text)) => text.is_empty(),
        Some(Content::Parts(_)) => false,
        None => true,
    };
    if !content_is_empty || message.name.is_some() {
        return false;
    }
    !ASSISTANT_FIELDS.iter().any(|field| {
        message
            .provider_specific_fields
            .get(*field)
            .is_some_and(|value| !matches!(value, Json::Null))
    })
}

// _handle_message_with_file turns file.file_id into the block's file_id and drops file
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

fn has_media_blocks(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|message| match &message.content {
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .any(|part| matches!(part.kind.as_str(), "image_url" | "file")),
        Some(MessageContent::Text(_)) | None => false,
    })
}

fn prompt_message(message: &ChatMessage) -> Result<Message, MistralChatError> {
    let value = serde_json::to_value(message)
        .map_err(|source| MistralChatError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| MistralChatError::MessageShape { source })
}

fn chat_message(message: &Message) -> Result<ChatMessage, MistralChatError> {
    let value = serde_json::to_value(message)
        .map_err(|source| MistralChatError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| MistralChatError::MessageShape { source })
}
