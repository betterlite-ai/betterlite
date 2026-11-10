use secrecy::SecretString;
use serde_json::Value;

use crate::chat::ChatMessage;
use crate::gigachat_chat::MODEL_PREFIX;
use crate::gigachat_chat::config::GigachatChatConfig;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_chat::headers::{HeaderInput, validate_environment};
use crate::gigachat_chat::messages::transform_messages;
use crate::gigachat_utils::get_api_base;
use crate::openai_base_config::{Headers, Object};

pub struct CompletionInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub api_base: Option<&'request str>,
    pub env_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub credentials: Option<&'request SecretString>,
    pub fallback_key: Option<&'request SecretString>,
    pub caller_headers: &'request Headers,
    pub optional_params: &'request Object,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub url: String,
    pub headers: Headers,
    pub body: Object,
}

pub async fn completion_request(
    config: &GigachatChatConfig<'_>,
    input: &CompletionInput<'_>,
) -> Result<CompletionRequest, GigaChatError> {
    let headers = validate_environment(
        &HeaderInput {
            api_key: input.api_key,
            credentials: input.credentials,
            fallback_key: input.fallback_key,
            caller_headers: input.caller_headers,
        },
        config.bearer,
    )
    .await?;
    let body = request_body(config, input).await?;
    Ok(CompletionRequest {
        url: format!(
            "{}/chat/completions",
            get_api_base(input.api_base, input.env_base)
        ),
        headers,
        body,
    })
}

async fn request_body(
    config: &GigachatChatConfig<'_>,
    input: &CompletionInput<'_>,
) -> Result<Object, GigaChatError> {
    let messages = transform_messages(input.messages, config.uploader).await?;
    let mut body = Object::new();
    body.insert(
        String::from("model"),
        Value::String(input.model.replace(MODEL_PREFIX, "")),
    );
    body.insert(String::from("messages"), Value::Array(messages));
    // LiteLLM copies these provider options only when the request carries them
    for key in [
        "temperature",
        "top_p",
        "max_tokens",
        "stream",
        "repetition_penalty",
        "profanity_check",
        "functions",
        "function_call",
    ] {
        if let Some(value) = input.optional_params.get(key) {
            body.insert(String::from(key), value.clone());
        }
    }
    Ok(body)
}
