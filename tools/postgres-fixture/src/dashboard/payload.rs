use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonPayload {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}
