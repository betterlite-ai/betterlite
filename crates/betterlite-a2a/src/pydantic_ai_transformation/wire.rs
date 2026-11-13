use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct TaskResponse {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub result: TaskResult,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct TaskResult {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub history: Vec<Message>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub message: Message,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct Artifact {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub parts: Vec<Part>,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub role: String,
    #[serde(default)]
    pub parts: Option<Vec<Part>>,
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Part {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, AgentJson>,
}

impl Part {
    #[must_use]
    pub fn text_content(&self) -> &str {
        if self.kind.as_deref() == Some("text") {
            self.text.as_deref().unwrap_or_default()
        } else {
            ""
        }
    }
}
