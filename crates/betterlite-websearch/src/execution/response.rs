use serde::Serialize;

use crate::call_transformation::native::ResultBlock;
use crate::records::{AnthropicSearchQuery, ServerToolUseType};

#[derive(Debug, Serialize)]
pub struct SyntheticSearchResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub message_kind: MessageKind,
    pub role: AssistantRole,
    pub model: String,
    pub content: Vec<SyntheticContentBlock>,
    pub stop_reason: StopReason,
    pub stop_sequence: Option<String>,
    pub usage: SearchUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantRole {
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SearchUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SyntheticContentBlock {
    ServerToolUse(ServerToolUseBlock),
    ToolResult(ResultBlock),
    Text(TextBlock),
}

#[derive(Debug, Serialize)]
pub struct ServerToolUseBlock {
    #[serde(rename = "type")]
    pub block_kind: ServerToolUseType,
    pub id: String,
    pub name: String,
    pub input: AnthropicSearchQuery,
}

#[derive(Debug, Serialize)]
pub struct TextBlock {
    #[serde(rename = "type")]
    pub block_kind: TextKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Text,
}
