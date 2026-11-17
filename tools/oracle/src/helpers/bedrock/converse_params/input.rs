use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MapParams(MapParamsInput),
    CachePoint(CachePointInput),
    TopK(TopKInput),
    ParallelConfig(ParallelConfigInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapParamsInput {
    pub(crate) model: String,
    pub(crate) model_cost: ParamMap,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) non_default_params: ParamMap,
    pub(crate) optional_params: ParamMap,
    pub(crate) global_drop: bool,
    pub(crate) call_drop: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachePointInput {
    pub(crate) model: Option<String>,
    pub(crate) model_cost: ParamMap,
    pub(crate) message_block: ParamMap,
    #[serde(default)]
    pub(crate) control: Option<ParamValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TopKInput {
    pub(crate) model: String,
    pub(crate) model_cost: ParamMap,
    pub(crate) inference_params: ParamMap,
    pub(crate) global_drop: bool,
    pub(crate) call_drop: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParallelConfigInput {
    pub(crate) additional_request_params: ParamMap,
    pub(crate) parallel_tool_use_config: ParamMap,
}
