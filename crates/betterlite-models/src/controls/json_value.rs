use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

// The extra_body passthrough and multipart form bodies carry provider-defined JSON, so the closed model below keeps that payload typed instead of reaching for serde_json::Value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
    Object(IndexMap<String, Self>),
}
