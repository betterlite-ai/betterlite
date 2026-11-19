use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

// One helper_case call: the Router the case builds, its cost map and the method it reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) model_cost: JsonObject,
    #[serde(default)]
    pub(crate) arguments: Arguments,
}

// Router(model_list=...) or get_router_model_info's supplied deployment: one deployment as the caller declares it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) litellm_params: Option<JsonObject>,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
}

// The method's own arguments.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Arguments {
    pub(crate) model_name: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) deployment: Option<Row>,
    pub(crate) received_model_name: Option<String>,
}

// The Router methods this operation reads, by the name the case calls.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Listing,
    Configured,
    RouterInfo,
    AzureLogs,
    DeploymentInfo,
    Raw,
}
