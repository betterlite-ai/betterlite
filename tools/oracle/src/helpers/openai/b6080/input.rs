use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Headers;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Supported {
        models: Vec<String>,
    },
    Embedding,
    Responses {
        cases: Vec<ResponsesCase>,
    },
    Decisions {
        cases: Vec<DecisionsCase>,
    },
    ErrorClass {
        cases: Vec<ErrorCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesCase {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_base: Option<String>,
    pub(crate) global_api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionsCase {
    pub(crate) cost: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCase {
    pub(crate) message: String,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: Headers,
}
