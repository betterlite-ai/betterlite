use serde::{Deserialize, Serialize};

// Helm `--set` and environment variables stringify everything, so a redis-py int/float/bool field can arrive as its real type or a digit string.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IntOrText {
    Int(i64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FloatOrText {
    Float(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BoolOrText {
    Bool(bool),
    Text(String),
}

// serde_json::Value is disallowed; an unmodeled redis-py kwarg is still a string, number or boolean, never nested.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PassthroughValue {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoercionKind {
    Int,
    Float,
    Bool,
}
