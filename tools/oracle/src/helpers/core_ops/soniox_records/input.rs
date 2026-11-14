use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Call {
    SupportedParams,
    MapParams {
        non_default_params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    ValidateEnvironment {
        headers: Headers,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    CompleteUrl {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Error {
        message: String,
        status_code: i64,
    },
    Response {
        payload: Object,
        #[serde(default)]
        response_format: Option<String>,
    },
}
