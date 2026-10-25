use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub(super) type AgentFlag = bool;
pub(super) type SecurityRequirements = Vec<BTreeMap<String, Vec<String>>>;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AgentJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}
