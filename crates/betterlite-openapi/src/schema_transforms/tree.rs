use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub type JsonObject = IndexMap<String, JsonTree>;

// JSON Schema is open-ended and LiteLLM rewrites $ref at any depth, so every keyword stays in the tree
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonTree {
    Null,
    Flag(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
    Object(JsonObject),
}
