use serde::{Deserialize, Serialize};

use crate::stream_base_assembly::response::{BaseResponse, ChoiceProviderFields};
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_content_assembly::wire::{AudioResponse, ThinkingBlock};
use crate::stream_tool_assembly::records::{FunctionOutput, ToolOutput};
use crate::stream_usage_assembly::wire::Usage;

pub const TEXT_OBJECT: &str = "text_completion";

// The base response carries the metadata and text; the extras are the fields only the assembler fills.
#[derive(Debug)]
pub(crate) struct Draft {
    pub(crate) base: BaseResponse,
    pub(crate) extras: Extras,
}

#[derive(Debug, Default)]
pub(crate) struct Extras {
    pub(crate) tool_calls: Option<Vec<ToolOutput>>,
    pub(crate) function_call: Option<FunctionOutput>,
    pub(crate) reasoning_content: Option<String>,
    pub(crate) thinking_blocks: Option<Vec<ThinkingBlock>>,
    pub(crate) annotations: Option<Vec<JsonValue>>,
    pub(crate) audio: Option<AudioResponse>,
    pub(crate) images: Option<Vec<JsonValue>>,
    pub(crate) provider_specific_fields: Option<JsonObject>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Response {
    Chat(Box<ChatResponse>),
    Text(TextResponse),
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    pub choices: Vec<ChatChoice>,
    pub usage: Usage,
    pub hidden_params: JsonObject,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ChatChoice {
    pub index: i64,
    pub message: ChatMessage,
    pub finish_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<ChoiceProviderFields>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    pub content: Option<String>,
    pub role: String,
    pub tool_calls: Option<Vec<ToolOutput>>,
    pub function_call: Option<FunctionOutput>,
    pub provider_specific_fields: Option<JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ThinkingBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<JsonValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<JsonValue>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<TextChoice>,
    pub usage: TextUsage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextChoice {
    pub text: String,
    pub index: i64,
    pub logprobs: Option<JsonValue>,
    pub finish_reason: Option<String>,
}

// TextCompletionResponse builds a full Usage, but only these three counts are ever filled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}
