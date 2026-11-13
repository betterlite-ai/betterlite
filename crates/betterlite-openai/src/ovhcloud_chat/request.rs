use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use crate::openai_base_config::ConfigFamily;

use super::{CHAT_ENDPOINT, DEFAULT_API_BASE, PROVIDER};

// OVHCloudChatConfig extends OpenAIGPTConfig, so the base parameter policy runs without the exact-OpenAI extensions
pub const CHAT_FAMILY: ConfigFamily = ConfigFamily::Other;

pub struct ChatTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

#[derive(Debug, Error)]
pub enum ChatRequestError {
    #[error("the OVHCloud extra_body value must be an object")]
    ExtraBody,
    #[error(transparent)]
    Content(#[from] ChatContentError),
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    format!(
        "{}/{CHAT_ENDPOINT}",
        api_base.map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'))
    )
}

// LiteLLM pops extra_body from the options, runs the OpenAI chat transform, then lays the map over the body
pub fn transform_request(request: &ChatTransform<'_>) -> Result<Object, ChatRequestError> {
    let mut optional_params = request.optional_params.clone();
    let extra_body = match optional_params.shift_remove("extra_body") {
        None => Object::new(),
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(ChatRequestError::ExtraBody);
        }
    };
    let mut body = ChatContent {
        model: request.model,
        messages: request.messages,
        optional_params: &optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: request.environment,
        downloads: request.downloads,
    }
    .transform(Mode::Sync)?
    .to_body()?;
    body.extend(extra_body);
    Ok(body)
}
