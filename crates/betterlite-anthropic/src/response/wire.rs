use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::response_content::wire::{AnthropicCompletionResponse, AnthropicToolCall, JsonFields};
use crate::usage::AnthropicUsage;

// LiteLLM's ModelResponse: no upstream id, a fixed object, and hidden params the transform replaces wholesale
#[derive(Debug, Clone, Serialize)]
pub struct ResponseEnvelope {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<Choice>,
    pub usage: Option<AnthropicUsage>,
    #[serde(rename = "_hidden_params")]
    pub hidden_params: JsonFields,
}

impl ResponseEnvelope {
    // ModelResponse() builds one default Choice and keeps the caller's generated id
    #[must_use]
    pub fn new(id: String) -> Self {
        Self {
            id,
            object: "chat.completion".to_owned(),
            created: 0,
            model: None,
            system_fingerprint: None,
            choices: vec![Choice::default()],
            usage: None,
            hidden_params: IndexMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub index: usize,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            index: 0,
            message: ChatMessage::default(),
            finish_reason: Some("stop".to_owned()),
        }
    }
}

// litellm.Message: role defaults to assistant, absent fields stay out of the dump, and content is written always
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<AnthropicToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<JsonFields>,
}

impl Default for ChatMessage {
    fn default() -> Self {
        Self {
            role: "assistant".to_owned(),
            content: None,
            tool_calls: None,
            thinking_blocks: None,
            reasoning_content: None,
            provider_specific_fields: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AnthropicReply {
    Success(AnthropicCompletionResponse),
    Error(Json),
}
