use serde::Serialize;
use serde_json::{Map, Value};

use crate::api::response_chat_history::reasoning::ReasoningRecord;
use crate::api::responses_tool_results::records::ToolCallChunk;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    #[default]
    Assistant,
    User,
    System,
    Tool,
    Function,
}

// Message's constructor turns a missing or empty role into assistant and validates the rest
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    pub content: Option<String>,
    pub role: ChatRole,
    pub tool_calls: Option<Vec<ToolCallChunk>>,
    pub function_call: Option<Value>,
    pub provider_specific_fields: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_items: Option<Vec<ReasoningRecord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatChoice {
    pub finish_reason: String,
    pub index: usize,
    pub message: ChatMessage,
}

// fields this conversion does not assign keep what the caller's record already held
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub created: i64,
    pub model: Option<String>,
    pub object: String,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    #[serde(skip)]
    pub hidden_params: Map<String, Value>,
}
