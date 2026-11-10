use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

pub mod auth;
pub mod error;
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
