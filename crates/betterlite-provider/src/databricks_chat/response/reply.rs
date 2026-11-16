use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::databricks_chat::content::ThinkingBlock;

// The OpenAI-shaped reply LiteLLM's transform_response builds, serialized the way model_dump writes it
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    pub id: String,
    pub created: i64,
    pub model: String,
    pub object: String,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    pub finish_reason: String,
    pub index: i64,
    pub message: ReplyMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<Object>,
}

// content, role, tool_calls and function_call are always written; the reasoning keys only when set
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplyMessage {
    pub content: Option<String>,
    pub role: String,
    pub tool_calls: Option<Vec<ReplyToolCall>>,
    pub function_call: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<Object>,
}

// ChatCompletionMessageToolCall keeps the call keys it does not name, such as index
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplyToolCall {
    #[serde(flatten)]
    pub extra: Object,
    pub function: ReplyFunction,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

// Function drops the function keys it does not name, including parameters
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplyFunction {
    pub arguments: String,
    pub name: Option<String>,
}

// Usage(**block): the three counts, the two detail blocks and the keys it forwards
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Usage {
    pub completion_tokens: i64,
    pub prompt_tokens: i64,
    pub total_tokens: i64,
    pub completion_tokens_details: Option<Object>,
    pub prompt_tokens_details: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
}
