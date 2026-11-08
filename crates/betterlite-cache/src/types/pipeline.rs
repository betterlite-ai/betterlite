use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedisPipelineIncrement {
    pub key: String,
    pub increment_value: f64,
    pub ttl: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedisPipelineSet {
    pub key: String,
    pub value: Json,
    pub ttl: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedisPipelineRpush {
    pub key: String,
    pub values: Vec<Json>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedisPipelineLpop {
    pub key: String,
    pub count: Option<i64>,
}
