use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::decisions::Json;
use crate::fireworks_catalog_metadata::{Catalog, FireworksMetadata};

pub type JsonObject = IndexMap<String, Json>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Stop {
    Text(String),
    List(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Name(String),
    Object(JsonObject),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextLimitBehavior {
    Error,
    Truncate,
}

// Names the LiteLLM adapter operation a selected config carries; selection itself never runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicModelInfo {
    Ollama,
    Lemonade,
}

// Adapters that only inherit the base provider and model-info methods carry no data of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatelessAdapter {
    OpenAi,
    Gemini,
    VertexAi,
    LitellmProxy,
    Topaz,
    Anthropic,
    Clarifai,
    EdenAi,
    Bedrock,
    Vllm,
}

impl StatelessAdapter {
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAIGPTConfig",
            Self::Gemini => "GeminiModelInfo",
            Self::VertexAi => "VertexAIModelInfo",
            Self::LitellmProxy => "LiteLLMProxyChatConfig",
            Self::Topaz => "TopazModelInfo",
            Self::Anthropic => "AnthropicModelInfo",
            Self::Clarifai => "ClarifaiConfig",
            Self::EdenAi => "EdenAIChatConfig",
            Self::Bedrock => "BedrockModelInfo",
            Self::Vllm => "VLLMModelInfo",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ProviderSpecificModelInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_function_calling: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_prompt_caching: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_pdf_input: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_web_search: Option<bool>,
}

// B668's OpenAIGPTConfig arguments are plain fields here, so an unknown constructor argument is rejected.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FireworksSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "n")]
    pub number: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Stop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_truncate_len: Option<i64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "context_length_exceeded_behavior"
    )]
    pub context_limit_behavior: Option<ContextLimitBehavior>,
}

impl FireworksSettings {
    // The base constructor runs first with its seven arguments, then the Fireworks-only ones are stored.
    #[must_use]
    pub fn with_args(mut self, args: Self) -> Self {
        self.frequency_penalty = args.frequency_penalty.or(self.frequency_penalty);
        self.max_tokens = args.max_tokens.or(self.max_tokens);
        self.number = args.number.or(self.number);
        self.stop = args.stop.or(self.stop);
        self.temperature = args.temperature.or(self.temperature);
        self.top_p = args.top_p.or(self.top_p);
        self.response_format = args.response_format.or(self.response_format);
        self.tools = args.tools.or(self.tools);
        self.tool_choice = args.tool_choice.or(self.tool_choice);
        self.top_k = args.top_k.or(self.top_k);
        self.presence_penalty = args.presence_penalty.or(self.presence_penalty);
        self.user = args.user.or(self.user);
        self.logprobs = args.logprobs.or(self.logprobs);
        self.reasoning_effort = args.reasoning_effort.or(self.reasoning_effort);
        self.prompt_truncate_len = args.prompt_truncate_len.or(self.prompt_truncate_len);
        self.context_limit_behavior = args.context_limit_behavior.or(self.context_limit_behavior);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LemonadeSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functions: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "n")]
    pub number: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Stop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Json>>,
}

impl LemonadeSettings {
    #[must_use]
    pub fn with_args(self, args: Self) -> Self {
        Self {
            repeat_penalty: args.repeat_penalty.or(self.repeat_penalty),
            functions: args.functions.or(self.functions),
            logit_bias: args.logit_bias.or(self.logit_bias),
            max_completion_tokens: args.max_completion_tokens.or(self.max_completion_tokens),
            max_tokens: args.max_tokens.or(self.max_tokens),
            number: args.number.or(self.number),
            presence_penalty: args.presence_penalty.or(self.presence_penalty),
            stop: args.stop.or(self.stop),
            temperature: args.temperature.or(self.temperature),
            top_p: args.top_p.or(self.top_p),
            top_k: args.top_k.or(self.top_k),
            response_format: args.response_format.or(self.response_format),
            tools: args.tools.or(self.tools),
        }
    }
}

#[derive(Debug, Default)]
pub struct FireworksAIConfig {
    pub settings: FireworksSettings,
    pub provider_info_cache: FireworksMetadata,
}

impl FireworksAIConfig {
    #[must_use]
    pub fn get_provider_info(
        &mut self,
        catalog: &Catalog,
        generation: u64,
        model: &str,
    ) -> ProviderSpecificModelInfo {
        let info = self
            .provider_info_cache
            .get_provider_info(catalog, generation, model);
        ProviderSpecificModelInfo {
            supports_function_calling: Some(info.supports_function_calling),
            supports_prompt_caching: Some(info.supports_prompt_caching),
            supports_reasoning: info.supports_reasoning,
            supports_vision: info.supports_vision,
            supports_pdf_input: info.supports_pdf_input,
            supports_web_search: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LemonadeChatConfig {
    pub settings: LemonadeSettings,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AzureFoundryModelInfo {
    pub model: Option<String>,
}
