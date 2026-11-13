mod tests;

use std::collections::BTreeMap;
use std::slice;

use serde::{Deserialize, Serialize};

use crate::pipeline::GuardrailPipeline;

const GLOBAL_SCOPE: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelPatterns {
    One(String),
    Many(Vec<String>),
}

impl ModelPatterns {
    #[must_use]
    pub fn patterns(&self) -> &[String] {
        match self {
            Self::One(pattern) => slice::from_ref(pattern),
            Self::Many(patterns) => patterns,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyCondition {
    pub model: Option<ModelPatterns>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyScope {
    pub teams: Option<Vec<String>>,
    pub keys: Option<Vec<String>>,
    pub models: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

impl PolicyScope {
    #[must_use]
    pub fn get_teams(&self) -> Vec<String> {
        match &self.teams {
            Some(teams) if !teams.is_empty() => teams.clone(),
            Some(_) | None => vec![GLOBAL_SCOPE.to_owned()],
        }
    }

    #[must_use]
    pub fn get_keys(&self) -> Vec<String> {
        match &self.keys {
            Some(keys) if !keys.is_empty() => keys.clone(),
            Some(_) | None => vec![GLOBAL_SCOPE.to_owned()],
        }
    }

    #[must_use]
    pub fn get_models(&self) -> Vec<String> {
        match &self.models {
            Some(models) if !models.is_empty() => models.clone(),
            Some(_) | None => vec![GLOBAL_SCOPE.to_owned()],
        }
    }

    #[must_use]
    pub fn get_tags(&self) -> Vec<String> {
        self.tags
            .as_ref()
            .map_or_else(Vec::<String>::new, Clone::clone)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyGuardrails {
    pub add: Option<Vec<String>>,
    pub remove: Option<Vec<String>>,
}

impl PolicyGuardrails {
    #[must_use]
    pub fn get_add(&self) -> Vec<String> {
        self.add
            .as_ref()
            .map_or_else(Vec::<String>::new, Clone::clone)
    }

    #[must_use]
    pub fn get_remove(&self) -> Vec<String> {
        self.remove
            .as_ref()
            .map_or_else(Vec::<String>::new, Clone::clone)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub inherit: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub guardrails: PolicyGuardrails,
    pub condition: Option<PolicyCondition>,
    pub pipeline: Option<GuardrailPipeline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyAttachment {
    pub policy: String,
    pub scope: Option<String>,
    pub teams: Option<Vec<String>>,
    pub keys: Option<Vec<String>>,
    pub models: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub priority: Option<i32>,
    #[serde(default)]
    pub default: bool,
}

impl PolicyAttachment {
    #[must_use]
    pub fn is_global(&self) -> bool {
        self.scope
            .as_deref()
            .is_some_and(|scope| scope == GLOBAL_SCOPE)
    }

    #[must_use]
    pub fn to_policy_scope(&self) -> PolicyScope {
        if self.is_global() {
            return PolicyScope {
                teams: Some(vec![GLOBAL_SCOPE.to_owned()]),
                keys: Some(vec![GLOBAL_SCOPE.to_owned()]),
                models: Some(vec![GLOBAL_SCOPE.to_owned()]),
                tags: None,
            };
        }
        PolicyScope {
            teams: self.teams.clone(),
            keys: self.keys.clone(),
            models: self.models.clone(),
            tags: self.tags.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    #[serde(default)]
    pub policies: BTreeMap<String, Policy>,
}
