use std::fmt;

use secrecy::SecretString;
use serde::Serialize;

use crate::chat::ChatMessage;
use crate::gigachat_chat::MODEL_PREFIX;
use crate::gigachat_chat::config::GigachatChatConfig;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_chat::headers::{HeaderInput, validate_environment};
use crate::gigachat_chat::messages::{GigachatMessage, transform_messages};
use crate::gigachat_utils::get_api_base;
use crate::openai_base_config::{Headers, Object};

const COPIED_OPTIONS: [&str; 8] = [
    "temperature",
    "top_p",
    "max_tokens",
    "stream",
    "repetition_penalty",
    "profanity_check",
    "functions",
    "function_call",
];

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestBody {
    model: String,
    messages: Vec<GigachatMessage>,
    #[serde(flatten)]
    options: Object,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub url: String,
    pub headers: Headers,
    pub body: RequestBody,
}

impl fmt::Debug for CompletionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompletionRequest")
            .field("url", &self.url)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("body", &self.body)
            .finish()
    }
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
    let messages = transform_messages(input.messages, config.uploader).await?;
    let options = COPIED_OPTIONS
        .into_iter()
        .filter_map(|key| {
            let value = input.optional_params.get(key)?;
            Some((String::from(key), value.clone()))
        })
        .collect();
    Ok(CompletionRequest {
        url: format!(
            "{}/chat/completions",
            get_api_base(input.api_base, input.env_base)
        ),
        headers,
        body: RequestBody {
            model: input.model.replace(MODEL_PREFIX, ""),
            messages,
            options,
        },
    })
}
