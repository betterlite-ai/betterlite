use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

// serde_json::Value is disallowed (clippy.toml); LiteLLM's JsonValue is this same tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonValue {
    #[default]
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(IndexMap<String, Self>),
}
