pub mod content;
pub mod deployment;
pub mod execution;
pub mod provider_error;
pub mod response;

use std::collections::BTreeMap;

use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::{MockPermission, MockPlan};
use betterlite_config::sections::mock_response::{
    MockFunctionCall, MockResponse, MockToolCall, UsageDefaults,
};
use betterlite_http::proxy::UpstreamResponse;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::chat::execution::ChatError;
use crate::chat::response::ChatResponse;
use crate::embedding::EmbeddingResponse;
use crate::text_completion_policy::reply::mapped_finish_reason;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use serde_with::skip_serializing_none;
use thiserror::Error;

// LiteLLM reads these request keys itself and never forwards them, and betterlite does not act on them yet
const LITELLM_REQUEST_KEYS: [&str; 14] = [
    "api_base",
    "api_key",
    "api_version",
    "base_url",
    "caching",
    "context_window_fallback_dict",
    "extra_headers",
    "fallbacks",
    "max_retries",
    "mock_delay",
    "mock_timeout",
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
pub type MockChatBuilder =
    fn(&MockPlan<'_>, UsageDefaults, &str, &str, u64) -> Result<ChatResponse, MockError>;
pub type MockEmbeddingBuilder =
    fn(Option<&MockResponse>, &str) -> Result<EmbeddingResponse, MockError>;
#[derive(Debug, Clone, Default)]
pub struct MockSettings {
    pub responses: BTreeMap<String, MockResponse>,
    pub usage: UsageDefaults,
    pub permission: MockPermission,
}
// the shared failure classifier maps this status to LiteLLM's type and code; no provider was called
#[must_use]
pub fn mock_failure(error: MockError) -> ChatError {
    if error == MockError::Timeout {
        return ChatError::Timeout;
    }
    ChatError::Provider {
        response: Box::new(UpstreamResponse {
            status: error.status().unwrap_or(500),
            headers: HeaderMap::new(),
            body: Vec::new(),
        }),
    }
}
// LiteLLM's Choices maps a given finish_reason and keeps the raw value when the mapping changed it
#[must_use]
pub fn choice_finish_reason(raw: Option<&str>) -> (String, BTreeMap<String, Value>) {
    let Some(given) = raw else {
        return (String::from("stop"), BTreeMap::new());
    };
    let mapped = mapped_finish_reason(given);
    let native = (mapped != given).then(|| {
        BTreeMap::from([(
            String::from("native_finish_reason"),
            Value::String(given.to_owned()),
        )])
    });
    (mapped.to_owned(), native.unwrap_or_default())
}
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatRequest {
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    pub stream_options: Option<StreamOptions>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub presence_penalty: Option<Number>,
    pub frequency_penalty: Option<Number>,
    #[serde(rename = "n")]
    pub choice_count: Option<u32>,
    pub max_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub seed: Option<i64>,
    pub stop: Option<Stop>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u32>,
    pub user: Option<String>,
    pub service_tier: Option<String>,
    pub reasoning_effort: Option<String>,
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
    pub parallel_tool_calls: Option<bool>,
    pub response_format: Option<ResponseFormat>,
    pub mock_response: Option<MockResponse>,
    pub mock_tool_calls: Option<Vec<ClientToolCall>>,
    // the mock plan reads arguments as text, so the client's object arguments are encoded at parse time
    #[serde(skip, default)]
    pub planned_tool_calls: Option<Vec<MockToolCall>>,
    // parameters betterlite does not model: LiteLLM forwards unknown ones and filters the OpenAI ones it knows
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}
// LiteLLM's Function accepts an argument string or an object it json-encodes, and ChatCompletionMessageToolCall fills the rest
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientToolCall {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: ClientFunctionCall,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientFunctionCall {
    pub name: String,
    pub arguments: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StreamOptions {
    #[serde(default)]
    pub include_usage: bool,
}

impl ChatRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RequestError> {
        let mut request: Self =
            serde_json::from_slice(bytes).map_err(|source| RequestError::Body { source })?;
        if let Some(name) = request
            .passthrough
            .keys()
            .find(|name| LITELLM_REQUEST_KEYS.contains(&name.as_str()))
        {
            return Err(RequestError::Unsupported { name: name.clone() });
        }
        request.planned_tool_calls = request.mock_tool_calls.as_deref().map(planned_tool_calls);
        Ok(request)
    }
}

// LiteLLM strips the provider prefix before mock_completion receives the model
pub(crate) fn mock_model(litellm_model: &str) -> &str {
    match litellm_model.split_once('/') {
        Some((_, model)) => model,
        None => litellm_model,
    }
}

fn planned_tool_calls(calls: &[ClientToolCall]) -> Vec<MockToolCall> {
    calls
        .iter()
        .map(|call| MockToolCall {
            id: call.id.clone(),
            kind: call.kind.clone(),
            function: MockFunctionCall {
                name: call.function.name.clone(),
                arguments: Some(match &call.function.arguments {
                    None => String::new(),
                    Some(Value::String(text)) => text.clone(),
                    Some(value) => value.to_string(),
                }),
            },
        })
        .collect()
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

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<MessageContent>,
    pub name: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
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

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Option<FunctionDefinition>,
    // fields of the other tool types, such as custom or web search tools
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

// parameters is a caller's JSON schema, free-form by definition
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
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

// json_schema carries a caller's JSON schema, free-form by definition
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
    pub json_schema: Option<Value>,
}

mod tests;
