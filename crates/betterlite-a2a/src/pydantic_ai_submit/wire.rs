use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;
use serde::Serialize;
use serde_with::{DefaultOnNull, serde_as};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: String,
    pub method: &'static str,
    pub params: RpcParams,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RpcParams {
    Send(BTreeMap<String, AgentJson>),
    Get { id: String },
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct TaskProgress {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub result: ProgressResult,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ProgressResult {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub id: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub status: ProgressStatus,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, strum::Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TaskState {
    Completed,
    Failed,
    Canceled,
    // Any other state keeps the poll loop going
    #[default]
    #[serde(other)]
    Other,
}

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ProgressStatus {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub state: TaskState,
}
