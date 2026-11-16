use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
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
pub(crate) struct AgentIdCall {
    pub(crate) model: String,
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsCall {
    // the agent route maps no parameter, so only optional_params is read
    #[serde(rename = "model")]
    pub(crate) _model: String,
    #[serde(rename = "non_default_params")]
    pub(crate) _non_default_params: Object,
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderInfoCall {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Object>,
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparationCall {
    pub(crate) model: String,
    pub(crate) api_base: String,
    pub(crate) api_key: Option<String>,
    pub(crate) optional_params: Object,
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidationCall {
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckCall {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) expected: Vec<u16>,
    pub(crate) error_msg: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    pub(crate) status: u16,
    pub(crate) message: String,
    // Python's get_error_class takes the headers and drops them; the answer already proves the drop
    #[serde(rename = "headers")]
    pub(crate) _headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PathFunction {
    Threads,
    Messages,
    ListMessages,
    Runs,
    RunStatus,
    ThreadAndRun,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathCall {
    pub(crate) function: PathFunction,
    pub(crate) api_base: String,
    pub(crate) api_version: String,
    pub(crate) thread_id: Option<String>,
    pub(crate) run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlagCall {
    pub(crate) model: String,
    pub(crate) stream: bool,
}
