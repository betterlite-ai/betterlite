pub mod ir;
pub mod openai;
pub mod systemone;
pub mod text;
pub mod validators;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub const MAX_DECISION_QUESTIONS: usize = 128;

pub type Extra = IndexMap<String, Json>;
pub type HiddenParams = IndexMap<String, Json>;

// numbers stay serde_json's Number because the workspace's arbitrary_precision breaks u64 and f64 under flatten
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct TokenCount(pub Number);

impl Default for TokenCount {
    fn default() -> Self {
        Self(Number::from(0_u8))
    }
}

// serde_json's Value sorts object keys unless a workspace-wide feature is on, and LiteLLM keeps them as sent
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DecisionsJson {
    Text(String),
    Object(IndexMap<String, Json>),
    List(Vec<Json>),
}
