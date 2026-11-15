use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::context::{PolicyMatchContext, ResolvedPolicy};
use crate::values::{Policy, PolicyGuardrails, PolicyScope};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyScopeResponse {
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl PolicyScopeResponse {
    #[must_use]
    pub fn from_scope(scope: &PolicyScope) -> Self {
        Self {
            teams: scope.get_teams(),
            keys: scope.get_keys(),
            models: scope.get_models(),
            tags: scope.get_tags(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyGuardrailsResponse {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

impl PolicyGuardrailsResponse {
    #[must_use]
    pub fn from_guardrails(guardrails: &PolicyGuardrails) -> Self {
        Self {
            add: guardrails.get_add(),
            remove: guardrails.get_remove(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyInfoResponse {
    pub policy_name: String,
    #[serde(default)]
    pub inherit: Option<String>,
    pub scope: PolicyScopeResponse,
    pub guardrails: PolicyGuardrailsResponse,
    pub resolved_guardrails: Vec<String>,
    pub inheritance_chain: Vec<String>,
}

impl PolicyInfoResponse {
    #[must_use]
    pub fn from_policy(policy_name: &str, policy: &Policy, resolved: &ResolvedPolicy) -> Self {
        Self {
            policy_name: policy_name.to_owned(),
            inherit: policy.inherit.clone(),
            scope: PolicyScopeResponse::default(),
            guardrails: PolicyGuardrailsResponse::from_guardrails(&policy.guardrails),
            resolved_guardrails: resolved.guardrails.clone(),
            inheritance_chain: resolved.inheritance_chain.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySummaryItem {
    #[serde(default)]
    pub inherit: Option<String>,
    pub scope: PolicyScopeResponse,
    pub guardrails: PolicyGuardrailsResponse,
    pub resolved_guardrails: Vec<String>,
    pub inheritance_chain: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyListResponse {
    pub policies: BTreeMap<String, PolicySummaryItem>,
    pub total_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyTestResponse {
    pub context: PolicyMatchContext,
    pub matching_policies: Vec<String>,
    pub resolved_guardrails: Vec<String>,
    #[serde(default)]
    pub message: Option<String>,
}
