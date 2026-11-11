use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::sections::settings::JsonValue;

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

// LiteLLM reads the first key of a chain entry, and a bare entry is its own model group
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FallbackChainEntry {
    Group(IndexMap<String, Vec<FallbackTarget>>),
    Target(FallbackTarget),
}
