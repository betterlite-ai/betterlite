use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2AMessage {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub role: Role,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub parts: Vec<AgentJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AgentJson>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Role(pub String);

impl Default for Role {
    fn default() -> Self {
        Self("user".to_owned())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<AgentJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AgentJson>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ModelResponse {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub choices: Vec<ModelChoice>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ModelChoice {
    #[serde(default)]
    pub message: Option<ModelMessage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ModelMessage {
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResponseMessage {
    pub kind: String,
    pub role: String,
    pub parts: Vec<TextPart>,
    #[serde(rename = "messageId")]
    pub message_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TextPart {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SendMessageResponse {
    pub jsonrpc: String,
    pub id: Option<String>,
    pub result: ResponseMessage,
}
