use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

// The sets are keyed by LiteLLM's attribute names; `vertex_ai` stands for `litellm.models_by_provider["vertex_ai"]`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelSetsInput {
    #[serde(default)]
    pub(super) vertex_ai: BTreeSet<String>,
    #[serde(flatten)]
    pub(super) sets: BTreeMap<String, BTreeSet<String>>,
}

// Each operation answers a list of cases in order, so one file can sweep every provider row.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidateEnvironment { cases: Vec<EnvironmentInput> },
    ValidModel { cases: Vec<ValidModelInput> },
    CheckValidKey { cases: Vec<KeyInput> },
    ShouldRetry { cases: Vec<StatusInput> },
    RetryAfterHeader { cases: Vec<HeaderInput> },
    CalculateRetryAfter { cases: Vec<BackoffInput> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    #[serde(default)]
    pub(super) environment: BTreeSet<String>,
    #[serde(default)]
    pub(super) model_sets: ModelSetsInput,
    pub(super) model: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidModelInput {
    #[serde(default)]
    pub(super) model_sets: ModelSetsInput,
    pub(super) model: String,
    pub(super) fails: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyInput {
    pub(super) model: String,
    pub(super) api_key: String,
    pub(super) fails: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusInput {
    pub(super) status_code: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderInput {
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BackoffInput {
    pub(super) remaining_retries: i32,
    pub(super) max_retries: i32,
    #[serde(default)]
    pub(super) min_timeout: i32,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) now: i64,
    pub(super) random: f64,
    pub(super) initial_retry_delay: Option<f64>,
    pub(super) max_retry_delay: Option<f64>,
    pub(super) jitter: Option<f64>,
}
