use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Number;

// serde_json::Number keeps numbers intact under the arbitrary_precision feature the workspace enables
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpValue {
    Object(BTreeMap<String, Self>),
    Null,
    Boolean(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
}

mod tests;
