use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use serde_with::skip_serializing_none;

use crate::api::responses_tool_namespaces::chat_tools::{ChatTool, WebSearchOptions};

use super::text_format::ResponseFormat;

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatCompletionRequest {
    pub messages: Vec<Value>,
    pub model: String,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<ChatTool>>,
    pub top_p: Option<Number>,
    pub user: Option<String>,
    pub temperature: Option<Number>,
    pub parallel_tool_calls: Option<bool>,
    pub max_tokens: Option<i64>,
    pub stream: Option<bool>,
    pub metadata: Option<Json>,
    pub service_tier: Option<String>,
    pub safety_identifier: Option<String>,
    pub web_search_options: Option<WebSearchOptions>,
    pub response_format: Option<ResponseFormat>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub context_management: Option<Json>,
    pub custom_llm_provider: Option<String>,
    pub extra_headers: Option<Json>,
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatCompletionSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_session_id: Option<String>,
}
