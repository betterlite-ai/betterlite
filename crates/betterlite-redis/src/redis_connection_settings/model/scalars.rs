use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// Helm `--set` and environment variables stringify everything, so a redis-py int/float/bool field can arrive as its real type or a digit string.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Scalar<T> {
    Value(T),
    Text(String),
}

pub type IntOrText = Scalar<i64>;
pub type FloatOrText = Scalar<f64>;
pub type BoolOrText = Scalar<bool>;

// serde_json::Value is disallowed, so a passthrough option that litellm forwards as a dict or list (socket_keepalive_options, retry_on_error) gets its own typed collection variant instead of rejecting deserialization outright.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PassthroughValue {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<PassthroughScalar>),
    Map(BTreeMap<String, PassthroughScalar>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PassthroughScalar {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoercionKind {
    Int,
    Bool,
}
