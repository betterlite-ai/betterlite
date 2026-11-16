use betterlite_bedrock::model_routes::ParamMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RemoveTtl {
        model_cost: ParamMap,
        cases: Vec<TtlCase>,
    },
    EnsureThinking {
        model_cost: ParamMap,
        cases: Vec<EnsureCase>,
    },
    SupportsExtendedThinking {
        model_cost: ParamMap,
        models: Vec<String>,
    },
    ClampEffort {
        model_cost: ParamMap,
        cases: Vec<ClampCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TtlCase {
    pub(crate) model: Option<String>,
    pub(crate) request: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnsureCase {
    pub(crate) model: String,
    pub(crate) request: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClampCase {
    pub(crate) model: String,
    pub(crate) params: ParamMap,
}
