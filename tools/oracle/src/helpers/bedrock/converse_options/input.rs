use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Nova2Models(Models),
    GptModels(Models),
    MinMaxTokens(Models),
    Handle(HandleCase),
    DeepseekFlags(ModelPairs),
    ThinkingAccepts(CatalogPairs),
    Sampling(CatalogModels),
    SupportedParams(CatalogModels),
    Clamp(ClampCase),
    WebSearch(Models),
    ClassDefaults(ClassDefaults),
    AutoToolChoice,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Models {
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandleCase {
    pub(crate) model_cost: ParamMap,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) model: String,
    pub(crate) effort: String,
    pub(crate) optional_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelPair {
    pub(crate) model: String,
    pub(crate) base_model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelPairs {
    pub(crate) cases: Vec<ModelPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogPairs {
    pub(crate) model_cost: ParamMap,
    pub(crate) cases: Vec<ModelPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogModels {
    pub(crate) model_cost: ParamMap,
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClampValues {
    pub(crate) optional_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClampCase {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) cases: Vec<ClampValues>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassDefaults {
    pub(crate) arguments: ParamMap,
}
