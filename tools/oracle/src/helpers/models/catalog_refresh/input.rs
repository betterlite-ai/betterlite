use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) replies: Vec<ReplyInput>,
    #[serde(default)]
    pub(crate) max_attempts: Option<u32>,
    #[serde(default)]
    pub(crate) timeout: Option<u64>,
    #[serde(default)]
    pub(crate) jitter: Vec<String>,
    #[serde(default)]
    pub(crate) probe_models: Vec<String>,
    #[serde(default)]
    pub(crate) probe_generalization: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    #[serde(default)]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Startup,
    Reload,
}
