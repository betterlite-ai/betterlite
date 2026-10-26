use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LassoRequestData {
    pub litellm_call_id: Option<String>,
    #[serde(default)]
    pub tools: Vec<OpenAiTool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiTool {
    pub function: Option<OpenAiFunction>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiFunction {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameters: Option<BTreeMap<String, Box<RawValue>>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCall {
    pub id: Option<String>,
    pub function: Option<ToolCallFunction>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallFields {
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<ToolArguments>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, Box<RawValue>>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LassoPayload {
    pub messages: Vec<Box<RawValue>>,
    #[serde(rename = "messageType")]
    pub message_type: MessageType,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "userId", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    pub source: Source,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Source {
    #[serde(rename = "type")]
    pub source_type: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum MessageType {
    #[serde(rename = "PROMPT")]
    Prompt,
    #[serde(rename = "COMPLETION")]
    Completion,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LassoResponse {
    #[serde(default)]
    pub violations_detected: bool,
    #[serde(default)]
    pub deputies: BTreeMap<String, bool>,
    #[serde(default)]
    pub findings: BTreeMap<String, Vec<Finding>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub action: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolArguments {
    Parsed(BTreeMap<String, Box<RawValue>>),
    Malformed { arguments: String },
}
