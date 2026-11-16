use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_with::rust::double_option;

use crate::databricks_chat::content::{DatabricksContent, DatabricksFunction};

// The response dict DatabricksResponse(**raw_response.json()) reads
#[derive(Debug, Deserialize)]
pub(super) struct DatabricksResponse {
    pub(super) id: String,
    pub(super) created: i64,
    #[serde(default)]
    pub(super) model: Option<String>,
    pub(super) choices: Vec<DatabricksChoice>,
    // Usage(**block) forwards every key it does not name, so the block stays open here
    pub(super) usage: Object,
}

#[derive(Debug, Deserialize)]
pub(super) struct DatabricksChoice {
    #[serde(default, with = "double_option")]
    pub(super) index: Option<Option<i64>>,
    pub(super) message: DatabricksMessage,
    #[serde(default, with = "double_option")]
    pub(super) finish_reason: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DatabricksMessage {
    #[serde(default, with = "double_option")]
    pub(super) content: Option<Option<DatabricksContent>>,
    #[serde(default)]
    pub(super) tool_calls: Option<Vec<DatabricksToolCall>>,
    #[serde(default, with = "double_option")]
    pub(super) reasoning_content: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    pub(super) reasoning: Option<Option<String>>,
}

// ChatCompletionMessageToolCall reads a function from every call and keeps the keys it does not name
#[derive(Debug, Deserialize)]
pub(super) struct DatabricksToolCall {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(rename = "type", default)]
    pub(super) kind: Option<String>,
    pub(super) function: DatabricksFunction,
    #[serde(flatten)]
    pub(super) extra: Object,
}
