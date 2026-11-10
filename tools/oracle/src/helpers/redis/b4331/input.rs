use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) caches: Vec<CacheSpec>,
    pub(crate) deadline_millis: u64,
    pub(crate) in_scope: bool,
    #[serde(default)]
    pub(crate) replies: BTreeMap<String, BTreeMap<String, Vec<Reply>>>,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheSpec {
    pub(crate) setting: String,
    pub(crate) namespace: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reply {
    Bulk(String),
    Nil,
    Array(Vec<Self>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostCall {
    pub(crate) cache: usize,
    pub(crate) key: String,
    pub(crate) value: f64,
    pub(crate) ttl: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Batch { cache: usize, keys: Vec<String> },
    PostCall { call: PostCall },
    PostCallOn { request: usize, call: PostCall },
    Await { handle: usize },
    FlushPostCall,
    Sleep { millis: u64 },
    Nested { cache: usize },
    Drain,
    CreateRequest,
    PrefetchArm { slot: String, value: String },
    PrefetchTake { slot: String },
    Snapshot,
}
