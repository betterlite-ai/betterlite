use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) calls: Vec<Call>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct PrepareArgs {
    pub(super) optional_params: Object,
    pub(super) headers: Headers,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Call {
    Prepare(PrepareArgs),
    CreateBody {
        prepare: PrepareArgs,
        model: String,
        file_id: Option<String>,
    },
    Redact {
        body: Object,
    },
    Raise {
        status: i64,
        action: String,
        body: String,
        #[serde(default)]
        headers: Headers,
        #[serde(default)]
        probes: Vec<String>,
    },
    PreCall {
        api_base: String,
        body: Object,
        failing: bool,
    },
    PostCall {
        audio: Option<String>,
        original_response: Object,
        body: Object,
        failing: bool,
    },
}
