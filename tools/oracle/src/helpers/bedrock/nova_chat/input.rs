use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    InlineCachePoints(InlineInput),
    FilterAllowedFields(TransformInput),
    ToolConfigInjection(TransformInput),
    Endpoint(EndpointInput),
    ConverseDelegation(DelegationInput),
    Defaults(DefaultsInput),
    ResponseModel(ResponseModelInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InlineInput {
    #[serde(default)]
    pub(crate) system: Vec<Json>,
    #[serde(default)]
    pub(crate) messages: Vec<Json>,
}

// The OpenAI request the Nova invoke transform starts from; B604 builds the Converse body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformInput {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) model_cost: ParamMap,
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
    #[serde(default)]
    pub(crate) litellm_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointInput {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    pub(crate) litellm_api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DelegationInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) model_cost: ParamMap,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) non_default_params: ParamMap,
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultsInput {
    #[serde(default)]
    pub(crate) arguments: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseModelInput {
    pub(crate) model: String,
    pub(crate) body: String,
}
