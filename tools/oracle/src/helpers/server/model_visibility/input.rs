use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

// One helper_case call: the router snapshot, the caller and the function the case reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) model_group_alias: JsonObject,
    #[serde(default)]
    pub(crate) arguments: Arguments,
}

// A row without model_info declares none, which no key of the mapping can match.
const fn default_model_info() -> Json {
    Json::Null
}

// Router(model_list=...): one deployment as the caller declares it, its model_info left as raw JSON for the non-mapping case.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) litellm_params: JsonObject,
    #[serde(default = "default_model_info")]
    pub(crate) model_info: Json,
}

// The call's own arguments; every one of them takes the value the source's signature gives an absent one.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Arguments {
    pub(crate) model_names: Vec<String>,
    pub(crate) user_role: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) healthy_only: Option<bool>,
    pub(crate) model_list_healthy_only: Option<bool>,
    pub(crate) with_router: Option<bool>,
    pub(crate) health: Health,
    pub(crate) broken_pattern: bool,
}

// B307's answer, injected: the names it reports unhealthy and whether the cache refuses to answer.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Health {
    pub(crate) unhealthy_names: Vec<String>,
    pub(crate) fails: bool,
}

// The owned entry points, by the name the case calls.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    UndiscoverableModelNames,
    DiscoverableRows,
    HiddenUnhealthyModelNames,
}
