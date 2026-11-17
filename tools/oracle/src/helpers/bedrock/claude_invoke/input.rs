use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

pub(super) type ModelCost = IndexMap<String, IndexMap<String, Json>>;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Input {
    Transform {
        model_cost: ModelCost,
        calls: Vec<TransformCall>,
    },
    MapParams {
        model_cost: ModelCost,
        calls: Vec<MapCall>,
    },
    Response {
        calls: Vec<ResponseCall>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransformCall {
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Json>,
    #[serde(default)]
    pub(super) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) fetch: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) inline: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MapCall {
    pub(super) model: String,
    #[serde(default)]
    pub(super) non_default_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) drop_params: bool,
}

#[derive(Deserialize)]
pub(super) struct ResponseCall {
    pub(super) raw_text: String,
    #[serde(default)]
    pub(super) messages: Vec<Json>,
    #[serde(default)]
    pub(super) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) now: i64,
}
