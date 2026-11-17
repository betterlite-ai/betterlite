use betterlite_models::decisions::{Extra, Json};
use serde::Serialize;

use crate::converse_content::ToolCall;
use crate::converse_content::citations::CitationAnnotation;
use crate::converse_content::recovery::MessageToolCall;
use crate::converse_usage::Usage;
use crate::converse_usage::reasoning::ThinkingBlock;

// The buffered chat-completion envelope LiteLLM fills in: the supplied response keeps its id, object and fingerprint, and the transform replaces the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub created: i64,
    pub model: String,
    pub object: String,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    pub finish_reason: String,
    pub index: i64,
    pub message: Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub content: Option<String>,
    pub role: String,
    pub tool_calls: Option<Vec<ReplyToolCall>>,
    pub function_call: Option<Json>,
    pub provider_specific_fields: Option<Extra>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<CitationAnnotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
}

// A content toolUse block keeps its block index while a recovered textual call has none, so the two wire shapes stay their own types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ReplyToolCall {
    Content(ToolCall),
    Recovered(MessageToolCall),
}

impl From<&ToolCall> for ReplyToolCall {
    fn from(tool: &ToolCall) -> Self {
        Self::Content(tool.clone())
    }
}

impl From<&MessageToolCall> for ReplyToolCall {
    fn from(tool: &MessageToolCall) -> Self {
        Self::Recovered(tool.clone())
    }
}
