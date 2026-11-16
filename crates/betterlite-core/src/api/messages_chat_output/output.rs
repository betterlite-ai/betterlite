use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::api::messages_context_clear::result::{AppliedEdit, UsageIteration};
use crate::api::messages_options::refusal::StopDetails;

// A provider compaction block travels with every key the provider sent, so it stays a raw object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ContentEntry {
    Block(ContentBlock),
    Compaction(Object),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Json,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_specific_fields: Option<SignatureFields>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignatureFields {
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ServerToolProjection {
    pub web_search_requests: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageProjection {
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tool_use: Option<ServerToolProjection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<Vec<UsageIteration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextManagement {
    pub applied_edits: Vec<AppliedEdit>,
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
pub enum AnthropicStopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    Refusal,
    Compaction,
}

// StopDetails carries no Clone, and nothing here copies a whole message.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct MessageResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: MessageKind,
    pub role: AssistantRole,
    pub model: String,
    pub stop_sequence: Option<String>,
    pub usage: UsageProjection,
    pub content: Vec<ContentEntry>,
    pub stop_reason: AnthropicStopReason,
    pub stop_details: Option<StopDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_management: Option<ContextManagement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageDelta {
    pub stop_reason: AnthropicStopReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeltaProjection {
    InputJsonDelta { partial_json: String },
    SignatureDelta { signature: String },
    ThinkingDelta { thinking: String },
    TextDelta { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamFrame {
    MessageDelta {
        delta: MessageDelta,
        usage: UsageProjection,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_management: Option<ContextManagement>,
    },
    ContentBlockDelta {
        index: i64,
        delta: DeltaProjection,
    },
}
