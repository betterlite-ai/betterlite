use betterlite_context::error::ContextError;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::azure::error::AzureError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod auth;
pub mod errors;
pub mod mapping;
pub mod messages;

mod tests;

// LiteLLM's list keeps the duplicate tools and tool_choice entries; a set projection would hide them
pub const SUPPORTED_OPENAI_PARAMS: [&str; 30] = [
    "temperature",
    "n",
    "stream",
    "stream_options",
    "stop",
    "max_tokens",
    "max_completion_tokens",
    "tools",
    "tool_choice",
    "presence_penalty",
    "frequency_penalty",
    "logit_bias",
    "user",
    "function_call",
    "functions",
    "tools",
    "tool_choice",
    "top_p",
    "logprobs",
    "top_logprobs",
    "response_format",
    "seed",
    "extra_headers",
    "parallel_tool_calls",
    "prediction",
    "modalities",
    "audio",
    "web_search_options",
    "prompt_cache_key",
    "store",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AzureChatSettings {
    pub frequency_penalty: Option<i64>,
    pub function_call: Option<Json>,
    pub functions: Option<Vec<Json>>,
    pub logit_bias: Option<Object>,
    pub max_tokens: Option<i64>,
    #[serde(rename = "n")]
    pub number: Option<i64>,
    pub presence_penalty: Option<i64>,
    pub stop: Option<Json>,
    pub temperature: Option<i64>,
    pub top_p: Option<i64>,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error(
        "Azure does not support 'tool_choice', for api_version={api_version}. Bump your API version to '2023-12-01-preview' or later. This parameter requires 'api_version=\"2023-12-01-preview\"' or later. Azure API Reference: https://learn.microsoft.com/en-us/azure/ai-services/openai/reference#chat-completions"
    )]
    ToolChoiceUnsupported { api_version: String },
    #[error(
        "Azure does not support 'required' as a tool_choice param, for api_version={api_version}. To drop 'tool_choice=required' for calls with this Azure API version, set `litellm.drop_params=True` or for proxy:\n\n`litellm_settings:\n drop_params: true`\nAzure API Reference: https://learn.microsoft.com/en-us/azure/ai-services/openai/reference#chat-completions"
    )]
    RequiredToolChoice { api_version: String },
    #[error("the api_version {field} is not an integer: {value}")]
    ApiVersionNumber { field: &'static str, value: String },
    #[error("the existing tools value is not a list")]
    ToolsNotList,
    #[error("response_format cannot be translated into a tool")]
    ResponseFormat {
        #[source]
        source: ContextError,
    },
    #[error("the messages cannot be converted for Azure")]
    Messages {
        #[source]
        source: AzureError,
    },
    #[error("a converted value cannot be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl PolicyError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::ToolChoiceUnsupported { .. } | Self::RequiredToolChoice { .. } => Some(400),
            Self::ApiVersionNumber { .. }
            | Self::ToolsNotList
            | Self::ResponseFormat { .. }
            | Self::Messages { .. }
            | Self::Encode { .. } => None,
        }
    }

    #[must_use]
    pub fn classify(&self) -> Option<ErrorClass> {
        match self {
            Self::ToolChoiceUnsupported { .. } | Self::RequiredToolChoice { .. } => {
                Some(ErrorClass::UnsupportedParams(Box::new(ErrorContext {
                    message: self.to_string(),
                    model: None,
                    provider: None,
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                })))
            }
            Self::ApiVersionNumber { .. }
            | Self::ToolsNotList
            | Self::ResponseFormat { .. }
            | Self::Messages { .. }
            | Self::Encode { .. } => None,
        }
    }
}
