use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Create,
    List,
}

// The only transport failure a case asks for: a request that runs past its read timeout
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    Timeout,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) operation: Operation,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) create_params: Map<String, Value>,
    #[serde(default)]
    pub(super) after: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<i64>,
    #[serde(default)]
    pub(super) order: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) timeout: Option<u64>,
    pub(super) status: u16,
    #[serde(default)]
    pub(super) reply_headers: BTreeMap<String, String>,
    pub(super) body: String,
    #[serde(default)]
    pub(super) failure: Option<Failure>,
}
