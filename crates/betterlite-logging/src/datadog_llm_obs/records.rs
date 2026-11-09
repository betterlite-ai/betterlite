use betterlite_models::decisions::Json;
use serde::Serialize;

use super::arguments::ToolArguments;
use super::values::Object;

// A tool call on a message, as LLM Obs names its fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: ToolArguments,
    pub tool_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

// The result of a tool call, as LLM Obs names its fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolResult {
    pub name: String,
    pub result: String,
    pub tool_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

// A tool the model was offered on the request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Object>,
}

// A message on a span, as LLM Obs names its fields; content stays as sent when it carries no text.
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

// The record LiteLLM writes for a message outside the declared dict shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputMessage {
    pub input: String,
}

// InputMeta.messages is a Message or a caller-supplied dict, so the conversion fallbacks ride along.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DdMessage {
    Message(Message),
    Input(InputMessage),
    Raw(Object),
}
