use betterlite_models::decisions::Json;
use serde::Deserialize;

// wrapper() drives the sync stub named completion; wrapper_async() drives the coroutine named acompletion
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    #[default]
    Sync,
    Async,
}

// One stub answer: the value it returns or the exception it raises
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Step {
    Response {
        value: Json,
    },
    Error {
        error_class: String,
        #[serde(default)]
        message: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct BudgetSpec {
    pub(super) max_budget: Option<serde_json::Number>,
    pub(super) current_cost: Option<serde_json::Number>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct RetrySpec {
    pub(super) num_retries: Option<i64>,
    pub(super) num_retries_per_request: Option<i64>,
}

// The presence of this object is litellm.cache being set; hit is the response the cache already holds
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct CacheSpec {
    pub(super) hit: Option<Json>,
}

// The request keys each stub call reports back, as values and as presence
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct ObserveSpec {
    pub(super) values: Vec<String>,
    pub(super) present: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(default)]
    pub(super) mode: Mode,
    pub(super) request: Json,
    #[serde(default)]
    pub(super) outcomes: Vec<Step>,
    #[serde(default)]
    pub(super) budget: BudgetSpec,
    #[serde(default)]
    pub(super) retry: RetrySpec,
    #[serde(default)]
    pub(super) cache: Option<CacheSpec>,
    #[serde(default)]
    pub(super) observe: ObserveSpec,
}
