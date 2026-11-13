use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Map, Value};
use serde_with::rust::double_option;

use crate::api::response_chat_history::reasoning::ReasoningRecord;

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";

// a TypedDict in LiteLLM, so its dump carries these five keys and no index
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PaddingChunk {
    pub text: String,
    pub tool_use: Option<Value>,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<Value>,
}

// ModelResponseStream.model_dump(), whose unset optional fields stay out of the dump
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatChunk {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    #[serde(with = "double_option", skip_serializing_if = "Option::is_none")]
    pub model: Option<Option<String>>,
    #[serde(with = "double_option", skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<Option<String>>,
    pub choices: Vec<ChatChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    pub provider_specific_fields: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatChoice {
    pub index: Option<i64>,
    pub delta: ChatDelta,
    pub finish_reason: Option<String>,
    pub logprobs: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// Delta.__init__ marks content, role, function_call, tool_calls and audio as set on every delta
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChatDelta {
    pub content: Option<String>,
    pub role: Option<String>,
    pub function_call: Option<Value>,
    pub tool_calls: Option<Vec<ChatToolCall>>,
    pub audio: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_items: Option<Vec<ReasoningRecord>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ChatCompletionToolCallChunk and ChatCompletionDeltaToolCall dump the same four declared fields
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatToolCall {
    pub id: Option<String>,
    pub index: Option<i64>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// the clock and id source of the iterator, injected the way B010 injects them into stream chunks
#[derive(Debug, Clone, Copy)]
pub struct ChunkFactory {
    pub identifier: fn() -> uuid::Uuid,
    pub clock: fn() -> i64,
}
