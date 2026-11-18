use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

// One helper_case call: the Router the case builds and the method it reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) model_group_alias: JsonObject,
    #[serde(default)]
    pub(crate) routing_groups: Vec<GroupRow>,
    #[serde(default)]
    pub(crate) inferred_provider: Option<String>,
    #[serde(default)]
    pub(crate) stale_index: bool,
    #[serde(default)]
    pub(crate) missing_ids: Vec<usize>,
    #[serde(default)]
    pub(crate) missing_info: Vec<usize>,
    #[serde(default)]
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

// Router(routing_groups=...): the fields of LiteLLM's RoutingGroup.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupRow {
    pub(crate) group_name: String,
    pub(crate) models: Vec<String>,
    pub(crate) routing_strategy: String,
}

// The method's own arguments; exclude_team_models keeps LiteLLM's false default when the case leaves it out.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Arguments {
    pub(crate) model_id: Option<String>,
    pub(crate) model_name: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) exclude_team_models: bool,
    pub(crate) candidate_id: Option<String>,
    pub(crate) deployment_index: Option<usize>,
}

// The Router methods this operation reads, by the name the case calls.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    GetDeployment,
    GetDeploymentByModelGroupName,
    GetModelIds,
    GetCandidateModelIdsForRoute,
    HasModelId,
    ResolveModelNameFromModelId,
    GetModelNames,
    GetModelList,
    GetModelListFromModelAlias,
    ShouldIncludeDeployment,
}
