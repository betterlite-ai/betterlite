use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::{ParamMap, ParamValue};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Input {
    MantleApiBase {
        region: String,
        cases: Vec<MantleCase>,
    },
    NativeResponses {
        model_cost: ParamMap,
        models: Vec<Option<String>>,
    },
    CachePointUnknownModel {
        model_cost: ParamMap,
        models: Vec<Option<String>>,
    },
    StrictToolsFallback {
        model_cost: ParamMap,
        models: Vec<String>,
    },
    ToolSearch {
        model_cost: ParamMap,
        models: Vec<String>,
    },
    EffortCeiling {
        model_cost: ParamMap,
        cases: Vec<EffortCase>,
    },
    ParallelTools {
        model_cost: ParamMap,
        models: Vec<Option<String>>,
    },
    ExtendedCacheTtl {
        model_cost: ParamMap,
        models: Vec<Option<String>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MantleCase {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) aws_bedrock_runtime_endpoint: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EffortCase {
    pub(super) model: String,
    pub(super) output_config: ParamValue,
}
