use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyMatchContext {
    pub team_alias: Option<String>,
    pub key_alias: Option<String>,
    pub model: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPolicy {
    pub policy_name: String,
    #[serde(default)]
    pub guardrails: Vec<String>,
    #[serde(default)]
    pub inheritance_chain: Vec<String>,
}
