use std::collections::BTreeMap;

use betterlite_models::reload::{DEFAULT_ATTEMPT_TIMEOUT, DEFAULT_MAX_ATTEMPTS};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) replies: Vec<ReplyInput>,
    #[serde(default = "default_max_attempts")]
    pub(crate) max_attempts: u32,
    #[serde(default = "default_timeout")]
    pub(crate) timeout: u64,
    #[serde(default)]
    pub(crate) jitter: Vec<String>,
    #[serde(default)]
    pub(crate) probe_models: Vec<String>,
    #[serde(default)]
    pub(crate) probe_generalization: Option<String>,
}

const fn default_max_attempts() -> u32 {
    DEFAULT_MAX_ATTEMPTS
}

const fn default_timeout() -> u64 {
    DEFAULT_ATTEMPT_TIMEOUT.as_secs()
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ReplyInput {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Startup,
    Reload,
}
