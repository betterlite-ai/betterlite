pub mod deployment;
pub mod execution;
pub mod provider_error;
pub mod response;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use thiserror::Error;

// LiteLLM reads these request keys itself and never forwards them, and betterlite does not act on them yet
const LITELLM_REQUEST_KEYS: [&str; 13] = [
    "api_base",
    "api_key",
    "api_version",
    "base_url",
    "caching",
    "context_window_fallback_dict",
    "extra_headers",
    "fallbacks",
    "max_retries",
    "mock_response",
    "num_retries",
    "stream_timeout",
    "timeout",
];

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("the request body is not a valid chat completion request: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite does not support the {name} request parameter yet")]
    Unsupported { name: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<Number>,
    #[serde(rename = "n", skip_serializing_if = "Option::is_none")]
    pub choice_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Stop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    // parameters betterlite does not model: LiteLLM forwards unknown ones and filters the OpenAI ones it knows
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

impl ChatRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RequestError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|source| RequestError::Body { source })?;
        if let Some(name) = request
            .passthrough
            .keys()
            .find(|name| LITELLM_REQUEST_KEYS.contains(&name.as_str()))
        {
            return Err(RequestError::Unsupported { name: name.clone() });
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
    Tool,
    Function,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    // message fields betterlite does not model, such as cache_control or audio, which LiteLLM forwards
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    // the fields of text, image, audio and file parts, which LiteLLM forwards as sent
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Stop {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionDefinition>,
    // fields of the other tool types, such as custom or web search tools
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    // a caller's JSON schema is free-form by definition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Named(NamedToolChoice),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NamedToolChoice {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionName,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FunctionName {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
    // a caller's JSON schema is free-form by definition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<Value>,
}

mod tests;
