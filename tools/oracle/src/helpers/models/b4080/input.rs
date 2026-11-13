use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
pub(crate) enum Input {
    Parse { raw: String },
    Target(TargetInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct TargetInput {
    #[serde(default)]
    pub(crate) router: Option<RouterInput>,
    pub(crate) queries: Vec<QueryInput>,
    #[serde(default)]
    pub(crate) dispatch: Option<DispatchInput>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RouterInput {
    pub(crate) served: Vec<ServedModels>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ServedModels {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QueryInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DispatchInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) params: IndexMap<String, Json>,
}
