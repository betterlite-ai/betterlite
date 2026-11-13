use std::collections::BTreeMap;

use betterlite_config::sections::agent_host_settings::Settings;
use betterlite_prompts::arguments::json::Object as PromptObject;
use betterlite_prompts::message_content::{
    Message as PromptMessage, convert_message_content_lists,
};
use secrecy::SecretString;
use serde_json::Value;
use thiserror::Error;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, RefuseDownloads};
use crate::openai_base_config::Object;

const PROVIDER: &str = "heroku";
const CHAT_COMPLETIONS_SUFFIX: &str = "/v1/chat/completions";

#[derive(Debug, Error)]
pub enum HerokuError {
    #[error("Heroku needs an API base: pass api_base or set HEROKU_API_BASE")]
    MissingBase,
    #[error(transparent)]
    Request(#[from] ChatContentError),
}

impl HerokuError {
    // HerokuError carries no status of its own; a content failure keeps the status the chat transform gave it
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingBase => None,
            Self::Request(error) => Some(error.status_code()),
        }
    }
}

pub struct HerokuChatConfig;

impl HerokuChatConfig {
    // LiteLLM has no default Heroku base, so an absent or empty resolved base is an error
    pub fn get_complete_url(
        settings: &Settings,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> Result<String, HerokuError> {
        let (resolved_base, _) = super::resolve_provider_info(
            settings.heroku_api_base.as_deref(),
            settings.heroku_api_key.as_ref(),
            None,
            api_base,
            api_key,
        );
        let Some(base) = resolved_base.filter(|value| !value.is_empty()) else {
            return Err(HerokuError::MissingBase);
        };
        if base.ends_with(CHAT_COMPLETIONS_SUFFIX) {
            return Ok(base);
        }
        Ok(format!("{base}{CHAT_COMPLETIONS_SUFFIX}"))
    }

    // HerokuChatConfig is an OpenAI subclass, so its async transform_request falls through to the sync one
    pub fn transform_request(
        model: &str,
        messages: &[ChatMessage],
        optional_params: &Object,
        mode: Mode,
    ) -> Result<PromptObject, HerokuError> {
        let converted = converted_messages(messages)?;
        let params: PromptObject = serde_json::from_value(Value::Object(optional_params.clone()))
            .map_err(|source| ChatContentError::MessageShape { source })?;
        let environment = BTreeMap::<String, String>::new();
        let downloads = RefuseDownloads;
        let content = ChatContent {
            model,
            messages: &converted,
            optional_params: &params,
            provider: Some(PROVIDER),
            api_base: None,
            global_api_base: None,
            openai_system_messages_first: false,
            environment: &environment,
            downloads: &downloads,
        };
        Ok(content.transform(mode)?.to_body()?)
    }
}

// B218's conversion runs before B503's message transform, so list content becomes text while the shape still matches LiteLLM's dicts
fn converted_messages(messages: &[ChatMessage]) -> Result<Vec<ChatMessage>, ChatContentError> {
    let mut prompt: Vec<PromptMessage> = Vec::with_capacity(messages.len());
    for message in messages {
        let value = serde_json::to_value(message)
            .map_err(|source| ChatContentError::MessageShape { source })?;
        prompt.push(
            serde_json::from_value(value)
                .map_err(|source| ChatContentError::MessageShape { source })?,
        );
    }
    let collapsed = convert_message_content_lists(&mut prompt);
    let mut typed: Vec<ChatMessage> = Vec::with_capacity(collapsed.len());
    for message in &collapsed {
        let value = serde_json::to_value(message)
            .map_err(|source| ChatContentError::MessageShape { source })?;
        typed.push(
            serde_json::from_value(value)
                .map_err(|source| ChatContentError::MessageShape { source })?,
        );
    }
    Ok(typed)
}
