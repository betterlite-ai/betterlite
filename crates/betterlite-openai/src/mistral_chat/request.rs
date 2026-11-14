use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{
    Message, convert_message_content_lists, strip_none_values_from_message,
};
use indexmap::IndexMap;
use serde_json::Value;

use super::messages::{
    handle_message_with_file, handle_name_in_message, handle_tool_call_message, has_media_blocks,
    is_empty_assistant_message, strip_output_only_fields,
};
use super::{MistralChatError, PROVIDER};
use crate::chat::{ChatMessage, ContentPart, MessageContent, Role};
use crate::chat_content::{ChatContent, Mode, TransformedRequest, UrlDownloader};
use crate::mistral_options::REASONING_SYSTEM_PROMPT;

const REASONING_FLAG: &str = "_add_reasoning_prompt";
const CLIENT_METADATA: &str = "client_metadata";

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

fn prepared_messages(messages: &[ChatMessage]) -> Result<Prepared, MistralChatError> {
    let stripped: Vec<ChatMessage> = messages.iter().map(strip_output_only_fields).collect();
    let media = has_media_blocks(&stripped);
    if media {
        return Ok(Prepared {
            messages: stripped,
            media,
        });
    }
    let mut prompt: Vec<Message> = stripped
        .iter()
        .map(prompt_message)
        .collect::<Result<Vec<_>, _>>()?;
    let converted = convert_message_content_lists(&mut prompt);
    let mut cleaned = Vec::with_capacity(converted.len());
    for message in converted {
        let message = handle_name_in_message(message);
        let message = handle_tool_call_message(message);
        if is_empty_assistant_message(&message) {
            continue;
        }
        cleaned.push(chat_message(&strip_none_values_from_message(&message))?);
    }
    Ok(Prepared {
        messages: cleaned,
        media,
    })
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
        updated.insert(
            0,
            ChatMessage {
                role: Role::System,
                content: Some(MessageContent::Text(prompt.to_owned())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                passthrough: IndexMap::new(),
            },
        );
        return updated;
    };
    if let Some(message) = updated.get_mut(index) {
        let content = message.content.take();
        message.content = Some(match content {
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
        });
    }
    updated
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
