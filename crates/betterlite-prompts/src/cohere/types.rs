use crate::arguments::json::Json;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CohereTool {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<CohereFunction>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CohereFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CohereCall {
    pub name: String,
    pub parameters: Json,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CohereToolResult {
    pub call: CohereCall,
    pub outputs: Vec<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CohereToolCall {
    pub name: Option<String>,
    pub parameters: Json,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "role")]
pub enum ChatHistoryEntry {
    #[serde(rename = "USER")]
    User { message: String },
    #[serde(rename = "SYSTEM")]
    System { message: String },
    #[serde(rename = "CHATBOT")]
    Chatbot {
        message: String,
        tool_calls: Vec<CohereToolCall>,
    },
    #[serde(rename = "TOOL")]
    Tool { tool_results: Vec<CohereToolResult> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CohereMessage {
    Text(String),
    ToolResult(CohereToolResult),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CohereV2Messages {
    pub message: CohereMessage,
    pub chat_history: Vec<ChatHistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoherePrompt {
    pub prompt: String,
    pub tool_results: Vec<CohereToolResult>,
}
