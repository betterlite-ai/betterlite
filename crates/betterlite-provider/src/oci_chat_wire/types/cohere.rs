use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum CohereApiFormat {
    #[default]
    #[serde(rename = "COHERE")]
    Cohere,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "role")]
pub enum CohereMessage {
    #[serde(rename = "USER")]
    User {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "CHATBOT")]
    Chatbot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(rename = "toolCalls", default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<CohereToolCall>>,
    },
    #[serde(rename = "SYSTEM")]
    System {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "TOOL")]
    Tool {
        #[serde(
            rename = "toolResults",
            default,
            skip_serializing_if = "Option::is_none"
        )]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereChatRequest {
    pub message: String,
    #[serde(default)]
    pub api_format: CohereApiFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_history: Option<Vec<CohereMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<CohereTool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preamble_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<Object>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_queries_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_entry_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounding: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_echo: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_search_queries_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_raw_prompting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_force_single_step: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_truncation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<Object>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereChatResponse {
    pub text: String,
    #[serde(default)]
    pub api_format: CohereApiFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_history: Option<Vec<CohereMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<CohereCitation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<Object>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_search_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_queries: Option<Vec<CohereSearchQuery>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<CohereToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CohereUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereChatResult {
    pub model_id: String,
    pub model_version: String,
    pub chat_response: CohereChatResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CohereStreamChunk {
    pub api_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_history: Option<Vec<CohereMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<CohereToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}
