use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;

use super::capture::Answer;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Path {
    Sync,
    Async,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResponseKind {
    ModelResponse,
    TextCompletion,
    Embedding,
    Image,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseInput {
    pub(super) kind: ResponseKind,
    pub(super) body: Json,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitScenario {
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) default_tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectScenario {
    pub(super) path: Path,
    pub(super) project_name: String,
    #[serde(default = "default_calls")]
    pub(super) calls: u32,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) default_tags: Vec<String>,
    #[serde(default)]
    pub(super) answers: Vec<Answer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogScenario {
    pub(super) path: Path,
    pub(super) start_time: String,
    pub(super) end_time: String,
    #[serde(default = "default_calls")]
    pub(super) calls: u32,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) default_tags: Vec<String>,
    #[serde(default)]
    pub(super) call_id: Option<Json>,
    #[serde(default)]
    pub(super) messages: Option<Json>,
    #[serde(default)]
    pub(super) standard_logging_object: Option<Json>,
    #[serde(default)]
    pub(super) metadata: Option<Json>,
    #[serde(default)]
    pub(super) response_cost: Option<Json>,
    #[serde(default)]
    pub(super) call_type: Option<String>,
    #[serde(default)]
    pub(super) response: Option<ResponseInput>,
    #[serde(default)]
    pub(super) api_call_start_time: Option<String>,
    #[serde(default)]
    pub(super) completion_start_time: Option<String>,
    #[serde(default)]
    pub(super) answers: Vec<Answer>,
}

const fn default_calls() -> u32 {
    1
}
