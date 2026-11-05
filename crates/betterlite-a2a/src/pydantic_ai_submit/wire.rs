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

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ProgressStatus {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub state: String,
}
