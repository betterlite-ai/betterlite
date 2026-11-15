use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::JsonObject;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineTestRequest {
    pub pipeline: JsonObject,
    pub test_messages: Vec<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyResolveRequest {
    #[serde(default)]
    pub team_alias: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyMatchDetail {
    pub policy_name: String,
    pub matched_via: String,
    #[serde(default)]
    pub guardrails_added: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyResolveResponse {
    #[serde(default)]
    pub effective_guardrails: Vec<String>,
    #[serde(default)]
    pub matched_policies: Vec<PolicyMatchDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentImpactResponse {
    #[serde(default)]
    pub affected_keys_count: i64,
    #[serde(default)]
    pub affected_teams_count: i64,
    #[serde(default)]
    pub unnamed_keys_count: i64,
    #[serde(default)]
    pub unnamed_teams_count: i64,
    #[serde(default)]
    pub sample_keys: Vec<String>,
    #[serde(default)]
    pub sample_teams: Vec<String>,
}
