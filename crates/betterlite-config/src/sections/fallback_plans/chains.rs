use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::sections::settings::JsonValue;

// one target of a fallback chain: a bare model group, or the group with the request parameters it overrides
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FallbackTarget {
    Model(String),
    Parameters(ParameterizedTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterizedTarget {
    pub model: String,
    // LiteLLM's fallback attempt merges these over the original request, so they stay as configured
    #[serde(flatten)]
    pub parameters: IndexMap<String, JsonValue>,
}

// one entry of a fallback chain: exactly one model group and its targets, or a bare target; a multi-key entry is a load error
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FallbackChainEntry {
    Group(IndexMap<String, Vec<FallbackTarget>>),
    Target(FallbackTarget),
}
