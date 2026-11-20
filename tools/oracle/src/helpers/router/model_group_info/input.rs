use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

// One helper_case call: the Router the case builds and the answer it reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model_group: String,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) model_group_alias: JsonObject,
    #[serde(default)]
    pub(crate) deployment_info: JsonObject,
    #[serde(default)]
    pub(crate) model_cost: JsonObject,
    #[serde(default)]
    pub(crate) parameters: JsonObject,
    #[serde(default)]
    pub(crate) lookup_error: bool,
    #[serde(default)]
    pub(crate) provider_error: ProviderError,
    #[serde(default)]
    pub(crate) missing_ids: Vec<usize>,
}

// The provider lookup failure a case forces inside the aggregation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderError {
    #[default]
    None,
    BadRequest,
    Other,
}

// Router(model_list=...): one deployment as the caller declares it, with the top-level fields LiteLLM keeps beside it.
#[derive(Debug, Deserialize)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    pub(crate) litellm_params: JsonObject,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
    #[serde(flatten)]
    pub(crate) extra: JsonObject,
}
