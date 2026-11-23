use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

// One b10047 call: the Router the case builds and the arguments get_model_access_groups takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model_list: Vec<Row>,
    pub(crate) model_group_alias: JsonObject,
    pub(crate) arguments: Arguments,
}

// Router(model_list=...): one deployment as the caller declares it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    pub(crate) litellm_params: JsonObject,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
}

// The method's own arguments; an absent one is None, as the method's default is.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Arguments {
    pub(crate) model_name: Option<String>,
    pub(crate) model_access_group: Option<String>,
    pub(crate) team_id: Option<String>,
}
