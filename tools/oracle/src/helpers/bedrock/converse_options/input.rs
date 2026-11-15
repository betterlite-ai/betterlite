use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Flags {
        kind: Flag,
        model_cost: ParamMap,
        models: Vec<String>,
    },
    Handle {
        model_cost: ParamMap,
        environment: BTreeMap<String, String>,
        model: String,
        effort: String,
        optional_params: ParamMap,
    },
    Params {
        model_cost: ParamMap,
        models: Vec<String>,
    },
    Clamp {
        environment: BTreeMap<String, String>,
        cases: Vec<ParamMap>,
    },
    Defaults {
        arguments: ParamMap,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Flag {
    Nova2,
    Gpt,
    MinMax,
    Sampling,
    WebSearch,
}
