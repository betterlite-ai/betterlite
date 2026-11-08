use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};

pub mod reply;

mod tests;

// Cohere v2 chat runs under LiteLLM's custom_llm_provider name for the OpenAI-format route
const PROVIDER: &str = "cohere_chat";

// LiteLLM advertises these names and maps only the renames below
pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "stream",
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "stop",
    "n",
    "tools",
    "tool_choice",
    "seed",
    "extra_headers",
];

// LiteLLM's CohereV2ChatConfig constructor declares these settings, and every one starts absent
const SETTINGS: &str = "preamble chat_history generation_id response_id conversation_id prompt_truncation connectors search_queries_only documents temperature max_tokens k p frequency_penalty presence_penalty tools tool_results seed";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CohereV2ChatConfig {
    settings: Object,
}

impl CohereV2ChatConfig {
    pub fn with_settings(settings: Object) -> Result<Self, CohereV2ConfigError> {
        if let Some(key) = settings
            .keys()
            .find(|key| !SETTINGS.split_whitespace().any(|name| name == key.as_str()))
        {
            return Err(CohereV2ConfigError::UnknownSetting { key: key.clone() });
        }
        Ok(Self { settings })
    }

    // a null setting is an absent one, as the project's compatibility policy says for every value
    #[must_use]
    pub fn get_config(&self) -> Object {
        self.settings
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        let target = match name.as_str() {
            "stream" | "temperature" | "frequency_penalty" | "presence_penalty" | "tools"
            | "seed" => name.as_str(),
            "n" => "num_generations",
            "top_p" => "p",
            "stop" => "stop_sequences",
            "max_completion_tokens" => "max_tokens",
            // max_completion_tokens wins over max_tokens wherever each one sits in the input
            "max_tokens" if !non_default_params.contains_key("max_completion_tokens") => {
                "max_tokens"
            }
            _ => continue,
        };
        optional_params.insert(target.to_owned(), value.clone());
    }
    optional_params
}

#[derive(Debug, Error)]
pub enum CohereV2ConfigError {
    #[error("Cohere v2 chat has no {key} setting")]
    UnknownSetting { key: String },
    #[error("api_base is required")]
    MissingApiBase,
}

pub fn complete_url(api_base: Option<&str>) -> Result<String, CohereV2ConfigError> {
    let Some(base) = api_base else {
        return Err(CohereV2ConfigError::MissingApiBase);
    };
    Ok(base.to_owned())
}

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub api_base: Option<&'request str>,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

pub fn request_body(request: &RequestInput<'_>) -> Result<Object, ChatContentError> {
    let content = ChatContent {
        model: request.model,
        messages: request.messages,
        optional_params: request.optional_params,
        provider: Some(PROVIDER),
        api_base: request.api_base,
        global_api_base: None,
        // LiteLLM reorders system messages only for the openai and azure providers
        openai_system_messages_first: false,
        environment: request.environment,
        downloads: request.downloads,
    };
    content.transform(Mode::Sync)?.to_body()
}
