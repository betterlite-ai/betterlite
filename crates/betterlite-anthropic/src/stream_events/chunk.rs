use betterlite_context::provider_message_primitives::FunctionKind;
use betterlite_models::decisions::Json;
use serde::Serialize;

use crate::stream_state::events::{ThinkingBlock, ToolFunctionChunk};
use crate::stream_state::wire::JsonFields;
use crate::usage::AnthropicUsage;

// LiteLLM builds exactly one choice at index 0 for an Anthropic event, so the chunk keeps that choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEventChunk {
    pub id: String,
    pub model: Option<String>,
    pub choices: Vec<StreamEventChoice>,
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEventChoice {
    pub index: i64,
    pub finish_reason: Option<String>,
    pub delta: StreamEventDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEventDelta {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamEventToolCall>>,
    // pydantic keeps provider_specific_fields in the dump even when it is null
    pub provider_specific_fields: Option<JsonFields>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEventToolCall {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: FunctionKind,
    pub function: ToolFunctionChunk,
    pub index: i64,
    // the caller survives only when content_block_start carried a value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<Json>,
}

#[derive(Debug, Default)]
pub(crate) struct Branch {
    pub(crate) content: String,
    pub(crate) tool_call: Option<StreamEventToolCall>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) usage: Option<AnthropicUsage>,
    pub(crate) provider_specific_fields: JsonFields,
    pub(crate) thinking_blocks: Option<Vec<ThinkingBlock>>,
    pub(crate) reasoning_content: Option<String>,
}
