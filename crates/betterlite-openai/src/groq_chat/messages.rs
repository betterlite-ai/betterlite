use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::Object as PromptObject;
use serde_json::Value;

use crate::chat::{ChatMessage, Role};
use crate::chat_content::{ChatContent, Mode, UrlDownloader};

use super::PROVIDER;
use super::error::GroqChatError;

pub struct MessageTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

// the B503 hook: Groq's null stripping runs first, then the shared OpenAI message transform
pub fn transform_messages(
    request: &MessageTransform<'_>,
) -> Result<Vec<ChatMessage>, GroqChatError> {
    let stripped = stripped_assistant_members(request.messages);
    let empty = PromptObject::new();
    let content = ChatContent {
        model: request.model,
        messages: &stripped,
        optional_params: &empty,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: request.environment,
        downloads: request.downloads,
    };
    match content.transform(Mode::Sync) {
        Ok(transformed) => Ok(transformed.messages),
        Err(error) => Err(GroqChatError::from(error)),
    }
}

pub struct RequestTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request serde_json::Map<String, Value>,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

// the hoisted message list reaches the shared request build, so every chat request carries the Groq tweak
pub fn transform_request(
    request: &RequestTransform<'_>,
) -> Result<serde_json::Map<String, Value>, GroqChatError> {
    let options: PromptObject =
        serde_json::from_value(Value::Object(request.optional_params.clone()))
            .map_err(|source| GroqChatError::Shape { source })?;
    let stripped = stripped_assistant_members(request.messages);
    let content = ChatContent {
        model: request.model,
        messages: &stripped,
        optional_params: &options,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: request.environment,
        downloads: request.downloads,
    };
    let body = content.transform(Mode::Sync)?.to_body()?;
    match serde_json::to_value(body).map_err(|source| GroqChatError::Shape { source })? {
        Value::Object(fields) => Ok(fields),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(GroqChatError::BodyShape)
        }
    }
}

fn stripped_assistant_members(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut stripped = Vec::with_capacity(messages.len());
    for message in messages {
        let mut message = message.clone();
        if message.role == Role::Assistant {
            message.passthrough.retain(|_, value| !value.is_null());
        }
        stripped.push(message);
    }
    stripped
}
