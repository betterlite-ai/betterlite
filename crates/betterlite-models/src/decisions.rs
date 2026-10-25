pub mod ir;
pub mod openai;
pub mod systemone;
pub mod text;
pub mod validators;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const MAX_DECISION_QUESTIONS: usize = 128;

pub type Extra = IndexMap<String, Json>;
pub type HiddenParams = IndexMap<String, Json>;

// serde_json's Value sorts object keys unless a workspace-wide feature is on, and LiteLLM keeps them as sent
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
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
