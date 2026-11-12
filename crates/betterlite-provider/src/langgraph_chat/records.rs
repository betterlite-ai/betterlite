use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LangGraphRole {
    Human,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphMessage {
    pub role: LangGraphRole,
    pub content: String,
    // A2A skill ids ride on the message metadata, so LiteLLM keeps it when it is not empty
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Object>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphInput {
    pub messages: Vec<LangGraphMessage>,
}

// LiteLLM copies these keys when they are present, even as null, so Json::Null is a value here
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphRequest {
    pub assistant_id: String,
    pub input: LangGraphInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_mode: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphStreamEvent {
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub data: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphResponseMessage {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphResponse {
    #[serde(default)]
    pub messages: Vec<LangGraphResponseMessage>,
    #[serde(default)]
    pub values: Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LangGraphParsedResponse {
    pub content: String,
    pub role: String,
    pub usage: Option<Usage>,
}
