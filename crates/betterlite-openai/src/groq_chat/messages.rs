use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::Object as PromptObject;

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
