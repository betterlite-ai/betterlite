use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) runs: Option<Vec<RunCall>>,
    pub(crate) dispatches: Option<Vec<DispatchCall>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Object>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_base: String,
    pub(crate) api_key: String,
    pub(crate) mode: Mode,
    pub(crate) supplied: bool,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Object>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_base: String,
    pub(crate) api_key: Option<String>,
    pub(crate) token: Option<String>,
    pub(crate) acompletion: bool,
    pub(crate) stream: bool,
    #[serde(default)]
    pub(crate) steps: Vec<Step>,
}
