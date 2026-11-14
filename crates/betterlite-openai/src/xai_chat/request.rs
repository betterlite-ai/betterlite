use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Message, strip_name_from_message};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode};
use crate::xai_chat::error::XaiChatError;

const ALLOWED_NAME_ROLES: [&str; 1] = ["user"];

// XAIChatConfig.transform_request: warn once for any web_search_options key, drop it, strip names, delegate
pub fn transform_request(content: &ChatContent<'_>) -> Result<Object, XaiChatError> {
    if content.optional_params.contains_key("web_search_options") {
        tracing::warn!(
            "XAI no longer supports web search on /chat/completions (Live Search is deprecated). Dropping 'web_search_options'. Use the Responses API for XAI web search."
        );
    }
    let mut optional_params = content.optional_params.clone();
    optional_params.shift_remove("web_search_options");
    let messages = stripped_names(content.messages)?;
    let parent = ChatContent {
        messages: &messages,
        optional_params: &optional_params,
        ..*content
    };
    Ok(parent.transform(Mode::Sync)?.to_body()?)
}

fn stripped_names(messages: &[ChatMessage]) -> Result<Vec<ChatMessage>, ChatContentError> {
    let allowed = vec![String::from(ALLOWED_NAME_ROLES[0])];
    let mut stripped = Vec::with_capacity(messages.len());
    for message in messages {
        let prompt = convert::<ChatMessage, Message>(message)?;
        let trimmed = strip_name_from_message(&prompt, &allowed);
        stripped.push(convert::<Message, ChatMessage>(&trimmed)?);
    }
    Ok(stripped)
}

fn convert<Source, Target>(message: &Source) -> Result<Target, ChatContentError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let value = serde_json::to_value(message)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| ChatContentError::MessageShape { source })
}
