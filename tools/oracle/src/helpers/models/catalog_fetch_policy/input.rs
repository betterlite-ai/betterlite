use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) model_keys: Vec<String>,
    #[serde(default)]
    pub(crate) backup_model_count: usize,
    #[serde(default)]
    pub(crate) min_model_count: Option<usize>,
    #[serde(default)]
    pub(crate) max_shrink_ratio: Option<String>,
    #[serde(default)]
    pub(crate) retry_after: Option<String>,
    #[serde(default)]
    pub(crate) attempt: u32,
    #[serde(default)]
    pub(crate) max_attempts: u32,
    #[serde(default)]
    pub(crate) jitter: Vec<String>,
    #[serde(default)]
    pub(crate) replies: Vec<ReplyInput>,
    #[serde(default)]
    pub(crate) timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    RawBlobRevision,
    ValidateMap,
    ClassifyResponse,
    ParseRetryAfter,
    RetryWait,
    NextRetryWait,
    FetchWithRetry,
}
