use betterlite_models::decisions::Extra;
use serde::Serialize;

use crate::converse_usage::Usage;

// LiteLLM always builds exactly one choice at index 0 for a Converse event, so the projection keeps that choice flat.
#[derive(Debug, Serialize)]
pub struct ConverseChunk {
    pub id: String,
    pub model: String,
    pub finish_reason: Option<String>,
    pub index: i64,
    pub delta: ConverseDelta,
    pub usage: Option<Usage>,
    pub provider_specific_fields: Extra,
}

#[derive(Debug, Serialize)]
pub struct ConverseDelta {
    pub content: Option<String>,
    pub role: Option<String>,
    pub tool_calls: Option<Vec<ToolCallChunk>>,
    pub provider_specific_fields: Option<Extra>,
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallChunk {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ToolFunctionChunk,
    pub index: i64,
}

#[derive(Debug, Serialize)]
pub struct ToolFunctionChunk {
    pub name: Option<String>,
    pub arguments: String,
}

// ChatCompletionThinkingBlock and ChatCompletionRedactedThinkingBlock, whose members LiteLLM sets one branch at a time.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingBlock {
    Thinking {
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        thinking: String,
    },
    Redacted {
        data: String,
    },
}
