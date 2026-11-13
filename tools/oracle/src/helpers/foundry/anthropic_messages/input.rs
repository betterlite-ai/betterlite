use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Url {
        api_bases: Vec<Option<String>>,
        environment: BTreeMap<String, String>,
    },
    Environment {
        calls: Vec<EnvironmentStep>,
    },
    Transform {
        request: Object,
    },
    Projection {
        request: Object,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Globals {
    pub(super) api_key: Option<String>,
    pub(super) azure_key: Option<String>,
}

// litellm_params keeps the dict LiteLLM received: a null api_key is present, a missing one is absent
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentStep {
    pub(super) headers: BTreeMap<String, String>,
    pub(super) litellm_params: BTreeMap<String, Option<String>>,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) optional_params: Object,
    pub(super) messages: Vec<Json>,
    pub(super) globals: Globals,
    pub(super) environment: BTreeMap<String, String>,
}
