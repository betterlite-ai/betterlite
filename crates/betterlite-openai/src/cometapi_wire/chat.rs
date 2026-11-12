use super::{CHAT_ENDPOINT, DEFAULT_API_BASE, PROVIDER};
use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use crate::openai_base_config::ConfigFamily;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;
// CometAPIConfig extends OpenAIGPTConfig, so the base parameter policy runs without the exact-OpenAI extensions
pub const CHAT_FAMILY: ConfigFamily = ConfigFamily::Other;
pub struct ChatTransform<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}
#[derive(Debug, Error)]
pub enum ChatError {
    #[error("the CometAPI extra_body value must be an object")]
    ExtraBody,
    #[error(transparent)]
    Content(#[from] ChatContentError),
}
// the endpoint test is a substring one, so a query value or a longer path that names chat/completions is kept
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base.map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'));
    if base.contains(CHAT_ENDPOINT) {
        return base.to_owned();
    }
    // LiteLLM looks for /v1 after the first // of the base, so /v10 or a query value counts as versioned
    if base.ends_with("/v1")
        || base
            .split_once("//")
            .map_or(base, |(_, rest)| rest)
            .contains("/v1")
    {
        format!("{base}/{CHAT_ENDPOINT}")
    } else {
        format!("{base}/v1/{CHAT_ENDPOINT}")
    }
}
// CometAPI's transform_request pops extra_body, runs the OpenAI chat transform, then lays the map over the body
pub fn transform_request(request: &ChatTransform<'_>) -> Result<Object, ChatError> {
    let mut optional_params = request.optional_params.clone();
    let extra_body = match optional_params.shift_remove("extra_body") {
        None => Object::new(),
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(ChatError::ExtraBody);
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
