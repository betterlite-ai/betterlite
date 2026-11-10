use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::skip_serializing_none;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum CohereApiFormat {
    #[default]
    #[serde(rename = "COHERE")]
    Cohere,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "role")]
pub enum CohereMessage {
    #[serde(rename = "USER")]
    User { message: Option<String> },
    #[serde(rename = "CHATBOT")]
    Chatbot {
        message: Option<String>,
        #[serde(rename = "toolCalls")]
        tool_calls: Option<Vec<CohereToolCall>>,
    },
    #[serde(rename = "SYSTEM")]
    System { message: Option<String> },
    #[serde(rename = "TOOL")]
    Tool {
        #[serde(rename = "toolResults")]
        tool_results: Option<Vec<CohereToolResult>>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CohereToolCall {
    pub name: String,
    pub parameters: Object,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CohereToolResult {
    pub call: CohereToolCall,
    pub outputs: Vec<Object>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CohereParameterDefinition {
    pub description: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "isRequired", default)]
    pub is_required: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereTool {
    pub name: String,
    pub description: String,
    pub parameter_definitions: BTreeMap<String, CohereParameterDefinition>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereChatRequest {
    pub message: String,
    #[serde(default)]
    pub api_format: CohereApiFormat,
    pub chat_history: Option<Vec<CohereMessage>>,
    pub max_tokens: Option<i64>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub top_k: Option<i64>,
    pub frequency_penalty: Option<Number>,
    pub presence_penalty: Option<Number>,
    pub stop_sequences: Option<Vec<String>>,
    pub seed: Option<i64>,
    pub tools: Option<Vec<CohereTool>>,
    pub response_format: Option<Object>,
    pub preamble_override: Option<String>,
    pub documents: Option<Vec<Object>>,
    pub search_queries_only: Option<bool>,
    pub search_entry_point: Option<String>,
    pub grounding: Option<Object>,
    pub is_echo: Option<bool>,
    pub is_search_queries_only: Option<bool>,
    pub is_raw_prompting: Option<bool>,
    pub is_force_single_step: Option<bool>,
    pub prompt_truncation: Option<String>,
    pub safety_mode: Option<String>,
    pub citation_quality: Option<String>,
    pub max_input_tokens: Option<i64>,
    pub is_stream: Option<bool>,
    pub stream_options: Option<Object>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<Object>,
    pub completion_tokens_details: Option<Object>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CohereCitation {
    pub start: i64,
    pub end: i64,
    pub text: String,
    pub document_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CohereSearchQuery {
    pub text: String,
    pub generation_id: String,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereChatResponse {
    pub text: String,
    #[serde(default)]
    pub api_format: CohereApiFormat,
    pub finish_reason: Option<String>,
    pub chat_history: Option<Vec<CohereMessage>>,
    pub citations: Option<Vec<CohereCitation>>,
    pub documents: Option<Vec<Object>>,
    pub error_message: Option<String>,
    pub is_search_required: Option<bool>,
    pub prompt: Option<String>,
    pub search_queries: Option<Vec<CohereSearchQuery>>,
    pub tool_calls: Option<Vec<CohereToolCall>>,
    pub usage: Option<CohereUsage>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereChatResult {
    pub model_id: String,
    pub model_version: String,
    pub chat_response: CohereChatResponse,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereStreamChunk {
    pub api_format: String,
    pub text: Option<String>,
    pub chat_history: Option<Vec<CohereMessage>>,
    pub finish_reason: Option<String>,
    pub tool_calls: Option<Vec<CohereToolCall>>,
    pub pad: Option<String>,
    pub index: Option<u32>,
}
