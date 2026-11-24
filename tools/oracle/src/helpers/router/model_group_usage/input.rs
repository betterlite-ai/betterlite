use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) model_group: Option<String>,
    #[serde(default)]
    pub(crate) now: i64,
    #[serde(default)]
    pub(crate) group_info: Option<GroupInfoInput>,
    #[serde(default)]
    pub(crate) cache: CacheInput,
    #[serde(default)]
    pub(crate) response: Option<JsonObject>,
    #[serde(default)]
    pub(crate) request_kwargs: Option<JsonObject>,
    #[serde(default)]
    pub(crate) metadata: Option<PrincipalInput>,
    #[serde(default)]
    pub(crate) litellm_metadata: Option<PrincipalInput>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) candidates: Option<Vec<usize>>,
}

// The owned Router method a case exercises.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Usage,
    IoUsage,
    Remaining,
    Response,
    Filter,
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

// The cached ModelGroupInfo a case hands the remaining-usage read.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupInfoInput {
    #[serde(default)]
    pub(crate) tpm: Option<i64>,
    #[serde(default)]
    pub(crate) rpm: Option<i64>,
    #[serde(default)]
    pub(crate) itpm: Option<i64>,
    #[serde(default)]
    pub(crate) otpm: Option<i64>,
}

// The scripted answer of one batch read: values in key order, a missing read, or a failed one.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheInput {
    #[serde(default)]
    pub(crate) mode: CacheMode,
    #[serde(default)]
    pub(crate) values: Vec<Json>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CacheMode {
    #[default]
    Missing,
    Values,
    Error,
}

// One user_api_key_auth view, as the filter reads its models and team_models grants.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrincipalInput {
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) team_models: Vec<String>,
}
