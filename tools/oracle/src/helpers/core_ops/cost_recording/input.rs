use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_cost: BTreeMap<String, BTreeMap<String, Json>>,
    #[serde(default)]
    pub(crate) vertex_location: Option<String>,
    #[serde(default)]
    pub(crate) ptu_cost_attribution: bool,
    #[serde(default)]
    pub(crate) logs: Vec<LogCase>,
    #[serde(default)]
    pub(crate) hidden_params: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) cost: Option<Json>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Record,
    SetResponseCost,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogCase {
    pub(crate) log: LogFields,
    pub(crate) calls: Vec<CallFields>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogFields {
    pub(crate) model: String,
    pub(crate) call_type: String,
    #[serde(default)]
    pub(crate) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) details: IndexMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallFields {
    pub(crate) response: ResponseSpec,
    pub(crate) calculator: CalculatorSpec,
    #[serde(default)]
    pub(crate) cache_hit: Option<bool>,
    #[serde(default)]
    pub(crate) litellm_model_name: Option<String>,
    #[serde(default)]
    pub(crate) router_model_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalculatorSpec {
    #[serde(default)]
    pub(crate) cost: Option<Json>,
    #[serde(default)]
    pub(crate) error: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ResponseSpec {
    Typed {
        model: String,
        usage: UsageFields,
        #[serde(default)]
        hidden_params: IndexMap<String, Json>,
        #[serde(default)]
        background: bool,
    },
    GenerateContent {
        body: Json,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UsageFields {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    #[serde(default)]
    pub(crate) total_tokens: Option<i64>,
}
