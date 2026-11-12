use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// a provider reply choice, of which _transform_choices reads the fields below
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplyChoice {
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub message: ReplyMessage,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplyMessage {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ReplyToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub function: Option<ReplyFunction>,
    #[serde(default)]
    pub custom: Option<ReplyCustom>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyCustom {
    pub name: String,
    pub input: String,
}

// LiteLLM's Choices, reduced to the fields _transform_choices can set
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransformedChoice {
    pub index: i64,
    pub finish_reason: String,
    pub message: ChoiceMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ChoiceToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CallFunction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallFunction {
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomPayload {
    pub name: String,
    pub input: String,
}

// ModelResponseStream's envelope; the delta stays an open object because providers add fields
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamReply {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub model: Option<String>,
    pub choices: Vec<StreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<StreamUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamChoice {
    pub index: i64,
    pub finish_reason: Option<String>,
    pub delta: StreamDelta,
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub function_call: Option<Value>,
    pub tool_calls: Option<Vec<Value>>,
    pub audio: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(flatten)]
    pub passthrough: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<Value>,
    #[serde(default)]
    pub completion_tokens_details: Option<Value>,
}
