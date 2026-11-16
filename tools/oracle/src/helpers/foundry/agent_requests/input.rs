use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Input {
    AgentIds(Vec<AgentIdCall>),
    Params(Vec<ParamsCall>),
    ProviderInfos(Vec<ProviderInfoCall>),
    CompleteUrls(Vec<UrlCall>),
    Requests(Vec<RequestCall>),
    Preparations(Vec<PreparationCall>),
    Validations(Vec<ValidationCall>),
    Checks(Vec<CheckCall>),
    Errors(Vec<ErrorCall>),
    Paths(Vec<PathCall>),
    Flags(Vec<FlagCall>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentIdCall {
    pub(super) model: String,
    pub(super) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamsCall {
    // the agent route maps no parameter, so only optional_params is read
    #[serde(rename = "model")]
    pub(super) _model: String,
    #[serde(rename = "non_default_params")]
    pub(super) _non_default_params: Object,
    pub(super) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderInfoCall {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlCall {
    pub(super) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestCall {
    pub(super) model: String,
    pub(super) messages: Vec<Object>,
    pub(super) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparationCall {
    pub(super) model: String,
    pub(super) api_base: String,
    pub(super) api_key: Option<String>,
    pub(super) optional_params: Object,
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationCall {
    pub(super) headers: BTreeMap<String, String>,
    pub(super) api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckCall {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) expected: Vec<u16>,
    pub(super) error_msg: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrorCall {
    pub(super) status: u16,
    pub(super) message: String,
    // Python's get_error_class takes the headers and drops them; the answer already proves the drop
    #[serde(rename = "headers")]
    pub(super) _headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PathFunction {
    Threads,
    Messages,
    ListMessages,
    Runs,
    RunStatus,
    ThreadAndRun,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PathCall {
    pub(super) function: PathFunction,
    pub(super) api_base: String,
    pub(super) api_version: String,
    pub(super) thread_id: Option<String>,
    pub(super) run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FlagCall {
    pub(super) model: String,
    pub(super) stream: bool,
}
