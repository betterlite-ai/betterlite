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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CohereV2ChatConfig {
    pub preamble: Option<Json>,
    pub chat_history: Option<Json>,
    pub generation_id: Option<Json>,
    pub response_id: Option<Json>,
    pub conversation_id: Option<Json>,
    pub prompt_truncation: Option<Json>,
    pub connectors: Option<Json>,
    pub search_queries_only: Option<Json>,
    pub documents: Option<Json>,
    pub temperature: Option<Json>,
    pub max_tokens: Option<Json>,
    pub top_k: Option<Json>,
    pub top_p: Option<Json>,
    pub frequency_penalty: Option<Json>,
    pub presence_penalty: Option<Json>,
    pub tools: Option<Json>,
    pub tool_results: Option<Json>,
    pub seed: Option<Json>,
}

impl CohereV2ChatConfig {
    #[must_use]
    pub fn get_config(&self) -> Object {
        let mut config = Object::new();
        for (key, value) in [
            ("preamble", &self.preamble),
            ("chat_history", &self.chat_history),
            ("generation_id", &self.generation_id),
            ("response_id", &self.response_id),
            ("conversation_id", &self.conversation_id),
            ("prompt_truncation", &self.prompt_truncation),
            ("connectors", &self.connectors),
            ("search_queries_only", &self.search_queries_only),
            ("documents", &self.documents),
            ("temperature", &self.temperature),
            ("max_tokens", &self.max_tokens),
            ("k", &self.top_k),
            ("p", &self.top_p),
            ("frequency_penalty", &self.frequency_penalty),
            ("presence_penalty", &self.presence_penalty),
            ("tools", &self.tools),
            ("tool_results", &self.tool_results),
            ("seed", &self.seed),
        ] {
            if let Some(setting) = value {
                config.insert(key.to_owned(), setting.clone());
            }
        }
        config
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
