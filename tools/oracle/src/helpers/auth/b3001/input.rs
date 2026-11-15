use std::collections::BTreeMap;

use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    MaxCost,
    InputCost,
    Count,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Deployment {
    pub(crate) model_name: String,
    pub(crate) backend_model: String,
    pub(crate) model_info: JsonObject,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) request: JsonObject,
    pub(crate) route: String,
    pub(crate) model_cost: BTreeMap<String, JsonObject>,
    #[serde(rename = "input_token_counts")]
    pub(crate) token_counts: Option<IndexMap<String, usize>>,
    pub(crate) model_list: Vec<Deployment>,
}
