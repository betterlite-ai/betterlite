use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct Response {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
    #[serde(rename = "id")]
    pub _id: Option<String>,
    #[serde(rename = "created")]
    pub _created: Option<i64>,
    #[serde(rename = "model")]
    pub _model: Option<String>,
    #[serde(rename = "system_fingerprint")]
    pub _system_fingerprint: Option<String>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct Choice {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[serde_as]
#[derive(Debug, Default, Deserialize)]
pub(super) struct Delta {
    pub content: Option<String>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub tool_calls: Vec<Call>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct Call {
    pub id: Option<String>,
    pub function: Function,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub index: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tool {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: Function,
    pub index: i64,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct Function {
    pub name: Option<String>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub arguments: String,
}

#[serde_as]
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct Usage {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub prompt_tokens: i64,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub completion_tokens: i64,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub total_tokens: i64,
}
