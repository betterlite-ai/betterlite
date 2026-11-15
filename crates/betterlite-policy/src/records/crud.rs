use serde::{Deserialize, Serialize};

use super::JsonObject;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyConditionRequest {
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyCreateRequest {
    pub policy_name: String,
    #[serde(default)]
    pub inherit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub guardrails_add: Option<Vec<String>>,
    #[serde(default)]
    pub guardrails_remove: Option<Vec<String>>,
    #[serde(default)]
    pub condition: Option<PolicyConditionRequest>,
    #[serde(default)]
    pub pipeline: Option<JsonObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyUpdateRequest {
    #[serde(default)]
    pub policy_name: Option<String>,
    #[serde(default)]
    pub inherit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub guardrails_add: Option<Vec<String>>,
    #[serde(default)]
    pub guardrails_remove: Option<Vec<String>>,
    #[serde(default)]
    pub condition: Option<PolicyConditionRequest>,
    #[serde(default)]
    pub pipeline: Option<JsonObject>,
}
