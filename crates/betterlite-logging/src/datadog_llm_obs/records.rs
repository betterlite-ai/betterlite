use betterlite_models::decisions::Json;
use serde::Serialize;

use super::arguments::ToolArguments;
use super::values::Object;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: ToolArguments,
    pub tool_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolResult {
    pub name: String,
    pub result: String,
    pub tool_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Object>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Message {
    pub role: String,
    pub content: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<Json>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<ToolResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputMessage {
    pub input: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DdMessage {
    Message(Message),
    Input(InputMessage),
    Raw(Object),
}
