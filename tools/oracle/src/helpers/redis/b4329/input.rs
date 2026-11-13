use std::collections::BTreeMap;

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) operation: Operation,
    pub(crate) namespace: Option<String>,
    pub(crate) redis_version: String,
    #[serde(default)]
    pub(crate) lists: BTreeMap<String, Vec<ListValue>>,
    pub(crate) key: Option<String>,
    #[serde(default)]
    pub(crate) values: Vec<ListValue>,
    pub(crate) max_len: Option<i64>,
    pub(crate) count: Option<i64>,
    #[serde(default)]
    pub(crate) operations: Vec<PipelineOperation>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Rpush,
    RpushAndTrim,
    RpushPipeline,
    Lpop,
    LpopPipeline,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PipelineOperation {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) values: Vec<ListValue>,
    pub(crate) count: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ListValue {
    Text(String),
    Bytes { bytes: Vec<u8> },
    Number(serde_json::Number),
}

impl ListValue {
    pub(crate) fn scalar(&self) -> RedisScalar {
        match self {
            Self::Text(text) => RedisScalar::Text(text.clone()),
            Self::Bytes { bytes } => RedisScalar::Bytes(bytes.clone()),
            Self::Number(number) => number.as_i64().map_or_else(
                || {
                    number
                        .as_f64()
                        .map_or_else(|| RedisScalar::Text(number.to_string()), RedisScalar::Float)
                },
                |integer| RedisScalar::Text(integer.to_string()),
            ),
        }
    }

    pub(crate) fn stored(&self) -> Vec<u8> {
        match self {
            Self::Text(text) => text.clone().into_bytes(),
            Self::Bytes { bytes } => bytes.clone(),
            Self::Number(number) => number.to_string().into_bytes(),
        }
    }
}
