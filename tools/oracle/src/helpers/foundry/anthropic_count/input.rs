use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_models::decisions::Json;
use serde::Deserialize;

use betterlite_oracle_helper_api::error::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Endpoint { api_bases: Vec<String> },
    Handle { cases: Vec<HandleCase> },
    Counter { cases: Vec<CounterCase> },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Globals {
    pub(crate) api_key: Option<String>,
    pub(crate) azure_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandleCase {
    pub(crate) model: String,
    pub(crate) messages: Json,
    pub(crate) api_key: String,
    pub(crate) api_base: String,
    #[serde(default)]
    pub(crate) system: Option<Json>,
    #[serde(default)]
    pub(crate) tools: Option<Json>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) request_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) litellm_params: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub(crate) globals: Globals,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CounterCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Option<Json>,
    #[serde(default)]
    pub(crate) contents: Option<Json>,
    #[serde(default)]
    pub(crate) system: Option<Json>,
    #[serde(default)]
    pub(crate) tools: Option<Json>,
    #[serde(default)]
    pub(crate) request_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) litellm_params: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub(crate) globals: Globals,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) request_model: Option<String>,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    pub(crate) body: String,
}

const fn default_status() -> u16 {
    200
}

pub(crate) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "Error".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
